//! Serialize a bounded transaction directly, without an intermediate JSON tree.

use std::io;
use std::io::Write;

use leptoskit_native::Mutation;

// A commit can replace both old and new trees. Allow JSON escaping and
// fixed-size fields beyond the Wasm host's 4 MiB property and 4096 node caps.
pub(super) const MAX_TRANSACTION_BYTES: usize = 64 * 1024 * 1024;

#[derive(serde::Serialize)]
struct Transaction<'a> {
	version: u32,
	root: Option<usize>,
	mutations: &'a [Mutation],
}

struct LimitedBuffer {
	bytes: Vec<u8>,
	limit: usize,
}

impl Write for LimitedBuffer {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if bytes.len() > self.limit - self.bytes.len() {
			return Err(io::Error::other("render transaction byte limit exceeded"));
		}

		self.bytes.extend_from_slice(bytes);
		Ok(bytes.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

pub(super) fn serialize(
	root: Option<usize>,
	mutations: &[Mutation],
) -> Result<Vec<u8>, serde_json::Error> {
	let mut buffer = LimitedBuffer {
		bytes: Vec::new(),
		limit: MAX_TRANSACTION_BYTES,
	};
	serde_json::to_writer(
		&mut buffer,
		&Transaction {
			version: 1,
			root,
			mutations,
		},
	)?;
	Ok(buffer.bytes)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mutation_serialization_counts_escaped_bytes_and_stops_at_the_limit() {
		let id = leptoskit_yoga::Yoga::new().create_node();
		let mutations = [Mutation::UpdateProps {
			id,
			props: leptoskit_native::NativeProps::text("\u{0}".repeat(100)),
		}];
		let transaction = Transaction {
			version: 1,
			root: Some(id.index()),
			mutations: &mutations,
		};
		let expected = serde_json::to_vec(&transaction).unwrap();
		let mut buffer = LimitedBuffer {
			bytes: Vec::new(),
			limit: expected.len(),
		};
		serde_json::to_writer(&mut buffer, &transaction).unwrap();
		assert_eq!(buffer.bytes, expected);
		let mut buffer = LimitedBuffer {
			bytes: Vec::new(),
			limit: expected.len() - 1,
		};
		assert!(serde_json::to_writer(&mut buffer, &transaction).is_err());
		assert!(buffer.bytes.len() <= buffer.limit);
	}
}
