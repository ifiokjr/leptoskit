//! One service for a trusted owner's deployment jobs and analytics history.

mod api;
mod dashboard;
mod store;
mod types;
mod worker;

pub use api::Service;
pub use store::Store;
pub use types::*;
pub use worker::Executor;
pub use worker::RunResult;

pub(crate) fn now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}

pub(crate) fn identifier(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 128
		&& value
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

pub(crate) fn random_key() -> anyhow::Result<String> {
	use std::fmt::Write as _;
	let mut bytes = [0; 32];
	getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("secure randomness is unavailable"))?;
	let mut key = String::with_capacity(64);
	for byte in bytes {
		write!(&mut key, "{byte:02x}")?;
	}
	Ok(key)
}
