//! Explicit launch argument snapshots with typed, non-exiting clap parsing.
//!
//! The host chooses which arguments to expose. No automatic runtime constructor
//! captures process arguments or discloses them to a guest.

use std::ffi::OsStr;
use std::ffi::OsString;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Empty request -> program string, argument count, then argument strings.
pub const SNAPSHOT: MethodId = MethodId(0);
/// Maximum argument count, excluding argv[0].
pub const MAX_ARGUMENTS: usize = 4096;
/// Maximum encoded argument snapshot.
pub const MAX_BYTES: usize = 1024 * 1024;

/// Snapshot creation and serialization failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("process arguments are unavailable on this platform")]
	Unavailable,
	#[error("argument snapshot exceeds configured protocol limits")]
	TooLarge,
	#[error("native argument is not valid UTF-8 and cannot cross the guest bridge")]
	NonUtf8,
	#[error("argument snapshot must include a program name")]
	MissingProgram,
	#[error("process arguments cannot contain NUL bytes")]
	InvalidArgument,
}

/// Immutable native arguments. Non-UTF-8 bytes are preserved for native
/// parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
	program: OsString,
	arguments: Vec<OsString>,
}
impl Snapshot {
	/// Capture once at application startup. Mobile and web return Unavailable.
	pub fn capture() -> Result<Self, Error> {
		#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
		{
			let mut args = std::env::args_os();
			let program = args.next().ok_or(Error::MissingProgram)?;
			let mut arguments = Vec::new();
			let mut length = program.as_encoded_bytes().len().saturating_add(8);
			for argument in args {
				length = length
					.saturating_add(argument.as_encoded_bytes().len())
					.saturating_add(4);
				if arguments.len() >= MAX_ARGUMENTS || length > MAX_BYTES {
					return Err(Error::TooLarge);
				}
				arguments.push(argument);
			}
			Self::new(program, arguments)
		}
		#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
		{
			Err(Error::Unavailable)
		}
	}

	/// Build an explicit snapshot, for host disclosure policies and activation
	/// events.
	pub fn new(program: OsString, arguments: Vec<OsString>) -> Result<Self, Error> {
		if program.is_empty() {
			return Err(Error::MissingProgram);
		}
		if program.as_encoded_bytes().contains(&0)
			|| arguments
				.iter()
				.any(|arg| arg.as_encoded_bytes().contains(&0))
		{
			return Err(Error::InvalidArgument);
		}
		let length = arguments.iter().fold(
			program.as_encoded_bytes().len().saturating_add(8),
			|length, arg| {
				length
					.saturating_add(arg.as_encoded_bytes().len())
					.saturating_add(4)
			},
		);
		if arguments.len() > MAX_ARGUMENTS || length > MAX_BYTES {
			return Err(Error::TooLarge);
		}
		Ok(Self { program, arguments })
	}

	/// argv[0], supplied by the caller. It is not a trusted executable path.
	pub fn program(&self) -> &OsStr {
		&self.program
	}

	/// Arguments after argv[0], in their original order.
	pub fn arguments(&self) -> &[OsString] {
		&self.arguments
	}

	/// Parse into the application's own typed clap model without exiting the
	/// process. Help/version requests and invalid input remain structured
	/// clap errors.
	pub fn parse<P: clap::Parser>(&self) -> Result<P, clap::Error> {
		P::try_parse_from(std::iter::once(&self.program).chain(&self.arguments))
	}

	/// Encode only valid UTF-8 arguments, without lossy conversion.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::default();
		output
			.bytes(self.program.to_str().ok_or(Error::NonUtf8)?.as_bytes())
			.map_err(|_| Error::TooLarge)?;
		output.u32(self.arguments.len() as u32);
		for argument in &self.arguments {
			output
				.bytes(argument.to_str().ok_or(Error::NonUtf8)?.as_bytes())
				.map_err(|_| Error::TooLarge)?;
		}
		Ok(output.finish())
	}

	/// Decode an exact, bounded snapshot. Truncation and trailing bytes are
	/// rejected.
	pub fn decode(payload: &[u8]) -> Result<Self, BridgeError> {
		if payload.len() > MAX_BYTES {
			return Err(BridgeError::TooLarge);
		}
		let mut decoder = Decoder::new(payload);
		let program = decoder.string()?;
		if program.is_empty() {
			return Err(BridgeError::InvalidArgument);
		}
		let count = decoder.u32()? as usize;
		if count > MAX_ARGUMENTS {
			return Err(BridgeError::TooLarge);
		}
		let mut arguments = Vec::new();
		for _ in 0..count {
			arguments.push(OsString::from(decoder.string()?));
		}
		decoder.finish()?;
		Self::new(program.into(), arguments).map_err(|_| BridgeError::InvalidArgument)
	}
}

/// Pre-encoded, explicitly disclosed startup arguments. Calls only clone Bytes.
#[derive(Debug)]
pub struct Arguments {
	encoded: Bytes,
}
impl Arguments {
	pub fn new(snapshot: &Snapshot) -> Result<Self, Error> {
		Ok(Self {
			encoded: snapshot.encode()?,
		})
	}
}
impl Plugin for Arguments {
	fn name(&self) -> &'static str {
		"arguments"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[SNAPSHOT]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, BridgeError> {
		if method != SNAPSHOT {
			return Err(BridgeError::Unavailable);
		}
		if !payload.is_empty() {
			return Err(BridgeError::InvalidArgument);
		}
		Ok(self.encoded.clone())
	}
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use super::*;
	/// Cached capability handle for the host's disclosed arguments.
	#[derive(Clone, Copy)]
	pub struct ArgumentsClient(leptoskit_bridge::guest::Client);
	impl ArgumentsClient {
		pub fn new() -> Result<Self, BridgeError> {
			leptoskit_bridge::guest::Client::new("arguments").map(Self)
		}

		pub async fn snapshot(self) -> Result<Snapshot, BridgeError> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[derive(Debug, PartialEq, clap::Parser)]
	struct Options {
		#[arg(long)]
		count: u32,
		#[arg(long)]
		verbose: bool,
		files: Vec<String>,
	}
	fn snapshot(args: &[&str]) -> Snapshot {
		Snapshot::new("app".into(), args.iter().map(OsString::from).collect()).unwrap()
	}
	#[test]
	fn typed_parse_respects_option_terminator_and_returns_help_errors() {
		let parsed = snapshot(&["--count=3", "--verbose", "--", "--file", "two words"])
			.parse::<Options>()
			.unwrap();
		assert_eq!(
			parsed,
			Options {
				count: 3,
				verbose: true,
				files: vec!["--file".into(), "two words".into()]
			}
		);
		assert_eq!(
			snapshot(&["--help"]).parse::<Options>().unwrap_err().kind(),
			clap::error::ErrorKind::DisplayHelp
		);
		assert!(snapshot(&["--count=no"]).parse::<Options>().is_err());
	}
	#[test]
	fn wire_snapshot_preserves_empty_unicode_and_order() {
		let original = snapshot(&["", "日本語", "one two", "--"]);
		assert_eq!(
			Snapshot::decode(&original.encode().unwrap()).unwrap(),
			original
		);
		let mut plugin = Arguments::new(&original).unwrap();
		assert_eq!(
			plugin.invoke(SNAPSHOT, Bytes::new()).unwrap(),
			original.encode().unwrap()
		);
		assert_eq!(
			plugin.invoke(SNAPSHOT, Bytes::from_static(b"bad")),
			Err(BridgeError::InvalidArgument)
		);
	}
	#[test]
	fn malformed_or_oversized_snapshots_are_rejected_before_large_allocation() {
		assert!(matches!(
			Snapshot::new("app".into(), vec!["a\0b".into()]),
			Err(Error::InvalidArgument)
		));
		let encoded = snapshot(&["x"]).encode().unwrap();
		for length in 0..encoded.len() {
			assert!(Snapshot::decode(&encoded[..length]).is_err());
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(Snapshot::decode(&trailing).is_err());
		let mut huge_count = Encoder::default();
		huge_count.bytes(b"app").unwrap();
		huge_count.u32(u32::MAX);
		assert_eq!(
			Snapshot::decode(&huge_count.finish()).unwrap_err(),
			BridgeError::TooLarge
		);
	}
	#[cfg(unix)]
	#[test]
	fn native_non_utf8_arguments_are_never_silently_replaced() {
		use std::os::unix::ffi::OsStringExt;
		let original = Snapshot::new("app".into(), vec![OsString::from_vec(vec![0xff])]).unwrap();
		assert!(matches!(original.encode(), Err(Error::NonUtf8)));
		assert_eq!(original.arguments()[0].as_encoded_bytes(), &[0xff]);
	}
}
