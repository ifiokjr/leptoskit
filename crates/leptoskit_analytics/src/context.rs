use serde::Deserialize;
use serde::Serialize;

/// Validated W3C version-00 context. Remote context provides correlation only;
/// a caller's sampled bit never controls our memory, collection or decisions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceContext(String);

impl TraceContext {
	#[must_use]
	pub fn parse(value: &str) -> Option<Self> {
		let bytes = value.as_bytes();
		if bytes.len() != 55 || &bytes[..3] != b"00-" || bytes[35] != b'-' || bytes[52] != b'-' {
			return None;
		}
		let hex = |slice: &[u8]| {
			slice
				.iter()
				.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
		};
		let nonzero = |slice: &[u8]| slice.iter().any(|b| *b != b'0');
		(hex(&bytes[3..35])
			&& hex(&bytes[36..52])
			&& hex(&bytes[53..55])
			&& nonzero(&bytes[3..35])
			&& nonzero(&bytes[36..52]))
		.then(|| Self(value.into()))
	}

	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	#[must_use]
	pub fn trace_id(&self) -> &str {
		&self.0[3..35]
	}

	#[must_use]
	pub fn span_id(&self) -> &str {
		&self.0[36..52]
	}

	#[cfg(feature = "server")]
	pub(crate) fn child(parent: Option<&Self>) -> Result<Self, getrandom::Error> {
		use std::fmt::Write as _;
		let mut random = [0; 24];
		loop {
			getrandom::fill(&mut random)?;
			if random[..16].iter().any(|b| *b != 0) && random[16..].iter().any(|b| *b != 0) {
				break;
			}
		}
		let mut value = String::from("00-");
		if let Some(parent) = parent {
			value.push_str(parent.trace_id());
		} else {
			for b in &random[..16] {
				let _ = write!(value, "{b:02x}");
			}
		}
		value.push('-');
		for b in &random[16..] {
			let _ = write!(value, "{b:02x}");
		}
		// This buffer is not an OTel trace exporter; do not claim it sampled a
		// distributed trace. Consumers can still use the correlation ID.
		value.push_str("-00");
		Ok(Self(value))
	}
}

impl TryFrom<String> for TraceContext {
	type Error = &'static str;

	fn try_from(value: String) -> Result<Self, Self::Error> {
		Self::parse(&value).ok_or("invalid W3C version-00 traceparent")
	}
}

impl From<TraceContext> for String {
	fn from(value: TraceContext) -> Self {
		value.0
	}
}
