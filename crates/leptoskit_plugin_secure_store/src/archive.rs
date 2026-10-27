//! One bounded credential is the source of truth, so crashes cannot stale
//! counters.

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
use super::*;

/// Cross-platform credential blob limit (Windows Credential Manager: 2560
/// bytes).
pub const MAX_ARCHIVE_BYTES: usize = 2560;

/// Host-owned aggregate limits, including names and encoding overhead.
#[derive(Debug, Clone, Copy)]
pub struct Quota {
	/// Maximum encoded store size, from 4 to 2560 bytes.
	pub max_bytes: usize,
	/// Maximum secrets, including empty values, at most 64.
	pub max_entries: usize,
}

impl Default for Quota {
	fn default() -> Self {
		Self {
			max_bytes: MAX_ARCHIVE_BYTES,
			max_entries: 64,
		}
	}
}

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
impl Quota {
	pub(crate) fn validate(self) -> Result<(), Error> {
		if !(4..=MAX_ARCHIVE_BYTES).contains(&self.max_bytes) || self.max_entries > 64 {
			return Err(Error::InvalidArgument);
		}

		Ok(())
	}
}

#[derive(Default)]
#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) struct Archive(Vec<(String, String, Vec<u8>)>);

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
impl Archive {
	pub(crate) fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() > MAX_ARCHIVE_BYTES {
			return Err(Error::TooLarge);
		}

		let mut input = Decoder::new(bytes);
		let count = input.u32()?;

		if count > 64 {
			return Err(Error::TooLarge);
		}

		let mut archive = Self::default();

		for _ in 0..count {
			let key = Key::new(input.string()?, input.string()?)?;
			let value = input.bytes()?;

			if value.len() > MAX_SECRET_BYTES || archive.get(key).is_some() {
				return Err(Error::InvalidArgument);
			}

			archive.0.push((
				key.service().to_owned(),
				key.key().to_owned(),
				value.to_vec(),
			));
		}

		input.finish()?;
		Ok(archive)
	}

	pub(crate) fn get(&self, key: Key<'_>) -> Option<&[u8]> {
		self.0
			.iter()
			.find(|(service, name, _)| service == key.service() && name == key.key())
			.map(|(_, _, value)| value.as_slice())
	}

	pub(crate) fn remove(&mut self, key: Key<'_>) {
		self.0
			.retain(|(service, name, _)| service != key.service() || name != key.key());
	}

	pub(crate) fn set(&mut self, key: Key<'_>, value: &[u8]) {
		self.remove(key);
		self.0.push((
			key.service().to_owned(),
			key.key().to_owned(),
			value.to_vec(),
		));
	}

	pub(crate) fn encode(&self, quota: Quota) -> Result<Bytes, Error> {
		quota.validate()?;

		if self.0.len() > quota.max_entries {
			return Err(Error::TooLarge);
		}

		let mut output = Encoder::default();
		output.u32(self.0.len() as u32);

		for (service, name, value) in &self.0 {
			output.bytes(service.as_bytes())?;
			output.bytes(name.as_bytes())?;
			output.bytes(value)?;
		}

		let bytes = output.finish();

		if bytes.len() > quota.max_bytes {
			return Err(Error::TooLarge);
		}

		Ok(bytes)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn quota_counts_empty_entries_names_and_overwrites_after_restart() {
		let quota = Quota {
			max_bytes: 64,
			max_entries: 2,
		};
		let a = Key::new("login", "a").unwrap();
		let b = Key::new("login", "b").unwrap();
		let c = Key::new("login", "c").unwrap();
		let mut archive = Archive::default();
		archive.set(a, b"1234");
		archive.set(b, b"");
		let saved = archive.encode(quota).unwrap();
		let mut restored = Archive::decode(&saved).unwrap();
		restored.set(c, b"");
		assert_eq!(restored.encode(quota), Err(Error::TooLarge));
		assert_eq!(
			Archive::decode(&saved).unwrap().get(a),
			Some(b"1234".as_slice())
		);
		restored.remove(c);
		restored.set(a, &[0; 40]);
		assert_eq!(restored.encode(quota), Err(Error::TooLarge));
		restored.remove(a);
		restored.set(c, b"value");
		assert!(restored.encode(quota).is_ok());
	}

	#[test]
	fn malformed_or_duplicate_records_fail_closed() {
		assert!(Archive::decode(&[0; MAX_ARCHIVE_BYTES + 1]).is_err());
		let key = Key::new("login", "token").unwrap();
		let mut archive = Archive::default();
		archive.set(key, b"secret");
		let bytes = archive.encode(Quota::default()).unwrap();

		for end in 0..bytes.len() {
			assert!(Archive::decode(&bytes[..end]).is_err());
		}

		let mut duplicate = bytes.to_vec();
		duplicate[0] = 2;
		duplicate.extend_from_slice(&bytes[4..]);
		assert!(Archive::decode(&duplicate).is_err());
	}
}
