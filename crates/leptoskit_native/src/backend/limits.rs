//! Count serialized batches before retaining another owned mutation copy.

use std::io;
use std::io::Write;

use super::Mutation;

pub(super) struct LogBudget {
	limit: usize,
	remaining: usize,
	exceeded: bool,
}

impl LogBudget {
	pub fn new(limit: usize) -> Self {
		Self {
			limit,
			remaining: limit,
			exceeded: false,
		}
	}

	pub fn admit(&mut self, mutations: &[Mutation]) -> bool {
		if mutations.is_empty() {
			return !self.exceeded;
		}

		if self.exceeded || serde_json::to_writer(&mut *self, mutations).is_err() {
			self.exceeded = true;

			return false;
		}

		true
	}

	pub fn exceeded(&self) -> bool {
		self.exceeded
	}

	pub fn restrict(&mut self, limit: usize) {
		let spent = self.limit - self.remaining;
		self.limit = self.limit.min(limit);
		self.remaining = self.limit.saturating_sub(spent);
		self.exceeded |= spent > self.limit;
	}

	pub fn drain(&mut self) {
		self.remaining = self.limit;
	}
}

impl Write for LogBudget {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if bytes.len() > self.remaining {
			return Err(io::Error::other("render mutation limit exceeded"));
		}

		self.remaining -= bytes.len();
		Ok(bytes.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::HeadlessBackend;
	use crate::MutationList;
	use crate::NativeBackend;

	#[test]
	fn draining_returns_capacity_but_cannot_revive_a_failed_recorder() {
		let id = leptoskit_yoga::Yoga::new().create_node();
		let mut batch = MutationList::new();
		batch.push(Mutation::UpdateLayout {
			id,
			x: 0.0,
			y: 0.0,
			width: 1.0,
			height: 1.0,
		});
		let size = serde_json::to_vec(batch.mutations()).unwrap().len();
		let mut backend = HeadlessBackend::new().with_log_limit(size);
		backend.apply(&batch);
		assert_eq!(backend.take_log().len(), 1);
		backend.apply(&batch);

		assert!(backend.render_error().is_none());
		backend.limit_mutation_log(size * 2);
		backend.apply(&batch);
		assert!(backend.render_error().is_some());
		assert_eq!(
			backend.take_log().len(),
			1,
			"an excessive batch is rejected atomically"
		);
		backend.limit_mutation_log(usize::MAX);
		backend.apply(&batch);
		assert_eq!(backend.log(), []);
		assert!(backend.render_error().is_some());
	}
}
