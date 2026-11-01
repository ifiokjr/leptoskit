//! Sign-In With Solana (SIWS): the text a wallet signs to prove it holds
//! an address, in the layout the Wallet Standard's `solana:signIn` writes
//! (its `createSignInMessageText`), following EIP-4361 and CAIP-122:
//!
//! ```text
//! notes.example wants you to sign in with your Solana account:
//! 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM
//!
//! Link this wallet to your account on Notes.
//!
//! URI: https://notes.example
//! Version: 1
//! Chain ID: mainnet
//! Nonce: 4WQ5WmPxpHdRqv8a3NhVF2
//! Issued At: 2026-10-30T12:00:00Z
//! Expiration Time: 2026-10-30T12:05:00Z
//! Request ID: link
//! ```
//!
//! There is one renderer, [`SiwsInput::message`], which the server, the
//! browser, and native apps share, and one strict parser,
//! [`SiwsMessage::parse`]: it accepts exactly what the renderer writes, so
//! `render(parse(text)) == text` for every text it accepts. The server
//! doesn't rely on the parser alone: it also compares the signed text,
//! byte for byte, with the message it issued.

use std::fmt;

use leptoskit_db::Pubkey;
use serde::Deserialize;
use serde::Serialize;

/// The first line's ending, after the domain.
const HEADER: &str = " wants you to sign in with your Solana account:";

/// The longest message accepted, in bytes. Real ones are a few hundred.
pub const MAX_MESSAGE_LENGTH: usize = 4096;

/// The longest value of a single field, in bytes.
const MAX_FIELD_LENGTH: usize = 512;

/// The labelled fields, in the order they appear.
const LABELS: [&str; 8] = [
	"URI: ",
	"Version: ",
	"Chain ID: ",
	"Nonce: ",
	"Issued At: ",
	"Expiration Time: ",
	"Not Before: ",
	"Request ID: ",
];

/// What a wallet is asked to sign: Sign-In With Solana's fields, as the
/// Wallet Standard's `solana:signIn` takes them (camelCase in JSON), so the
/// browser hands it to the wallet as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiwsInput {
	/// The site asking, as `host` or `host:port`. Wallets warn when it isn't
	/// the page they're asked from, which is what stops a phishing site
	/// relaying the message.
	pub domain: String,
	/// The address that must sign, when it's known already, as when
	/// re-proving a linked wallet. Otherwise the wallet fills in the one the
	/// person picks.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub address: Option<String>,
	/// What signing means, for people: one line.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub statement: Option<String>,
	/// The site's origin.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub uri: Option<String>,
	/// Always `1`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub version: Option<String>,
	/// The cluster, such as `mainnet`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub chain_id: Option<String>,
	/// A single-use random value: the server's challenge.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nonce: Option<String>,
	/// When the server issued it, in RFC 3339.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub issued_at: Option<String>,
	/// When it stops working, in RFC 3339.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expiration_time: Option<String>,
	/// When it starts working, in RFC 3339.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub not_before: Option<String>,
	/// What it's for, such as `link`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request_id: Option<String>,
	/// Links the person is asked to trust, one per line.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub resources: Vec<String>,
}

impl SiwsInput {
	/// The exact text a wallet signs for this input and `address`, as the
	/// Wallet Standard's `solana:signIn` writes it. A wallet that only offers
	/// `solana:signMessage` signs this text itself.
	pub fn message(&self, address: &Pubkey) -> String {
		let mut message = format!("{}{HEADER}\n{address}", self.domain);

		if let Some(statement) = &self.statement {
			message.push_str("\n\n");
			message.push_str(statement);
		}

		let mut fields = Vec::new();
		let values = [
			&self.uri,
			&self.version,
			&self.chain_id,
			&self.nonce,
			&self.issued_at,
			&self.expiration_time,
			&self.not_before,
			&self.request_id,
		];

		for (label, value) in LABELS.iter().zip(values) {
			if let Some(value) = value {
				fields.push(format!("{label}{value}"));
			}
		}

		if !self.resources.is_empty() {
			fields.push("Resources:".to_owned());
			fields.extend(
				self.resources
					.iter()
					.map(|resource| format!("- {resource}")),
			);
		}

		if !fields.is_empty() {
			message.push_str("\n\n");
			message.push_str(&fields.join("\n"));
		}

		message
	}
}

/// A Sign-In With Solana message, parsed strictly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiwsMessage {
	/// The address that signed it.
	pub address: Pubkey,
	/// Every other field.
	pub input: SiwsInput,
}

/// Why a message isn't a Sign-In With Solana message. Which rule it broke
/// is for tests and logs; clients only hear that the wallet's answer didn't
/// check out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiwsError {
	/// Longer than [`MAX_MESSAGE_LENGTH`].
	TooLong,
	/// A character no field may hold, such as a carriage return or a control
	/// character.
	Character,
	/// The first line isn't `<domain> wants you to sign in with your Solana
	/// account:`, or the domain isn't a host.
	Header,
	/// The second line isn't a Solana address, spelled as wallets spell it.
	Address,
	/// Paragraphs aren't separated by one blank line, or there are too many.
	Layout,
	/// A statement that's empty or looks like a field.
	Statement,
	/// A field that's unknown, repeated, out of order, or empty.
	Field,
	/// `Version` isn't `1`.
	Version,
	/// The nonce isn't at least 8 letters and digits.
	Nonce,
	/// A time isn't RFC 3339.
	Time,
}

impl fmt::Display for SiwsError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::TooLong => "the message is too long",
			Self::Character => "the message has a character it can't",
			Self::Header => "the first line isn't a sign-in request from a domain",
			Self::Address => "the second line isn't a Solana address",
			Self::Layout => "the message isn't laid out as Sign-In With Solana",
			Self::Statement => "the statement is empty or looks like a field",
			Self::Field => "a field is unknown, repeated, out of order, or empty",
			Self::Version => "the version isn't 1",
			Self::Nonce => "the nonce isn't at least 8 letters and digits",
			Self::Time => "a time isn't in RFC 3339",
		})
	}
}

impl std::error::Error for SiwsError {}

impl SiwsMessage {
	/// Parses `text`, accepting exactly the layout [`SiwsInput::message`]
	/// writes: the header, the address, an optional one-line statement, and
	/// the fields in their order, each at most once, separated by single
	/// newlines, with nothing trailing.
	///
	/// # Errors
	///
	/// The [`SiwsError`] for the first rule `text` breaks.
	pub fn parse(text: &str) -> Result<Self, SiwsError> {
		if text.len() > MAX_MESSAGE_LENGTH {
			return Err(SiwsError::TooLong);
		}

		if text
			.chars()
			.any(|character| character.is_control() && character != '\n')
		{
			return Err(SiwsError::Character);
		}

		let mut paragraphs = text.split("\n\n");
		let head = paragraphs.next().unwrap_or_default();
		let (domain_line, address) = head.split_once('\n').ok_or(SiwsError::Header)?;
		let domain = domain_line
			.strip_suffix(HEADER)
			.filter(|domain| valid_domain(domain))
			.ok_or(SiwsError::Header)?;
		// The address is the whole line, spelled exactly: no spaces around
		// it, no second line.
		let parsed = Pubkey::parse(address).map_err(|_| SiwsError::Address)?;

		if parsed.as_inner() != address {
			return Err(SiwsError::Address);
		}

		let mut input = SiwsInput {
			domain: domain.to_owned(),
			..SiwsInput::default()
		};
		let rest: Vec<&str> = paragraphs.collect();

		let (statement, fields) = match rest.as_slice() {
			[] => (None, None),

			[only] if is_fields(only) => (None, Some(*only)),
			[only] => (Some(*only), None),
			[statement, fields] => (Some(*statement), Some(*fields)),
			_ => return Err(SiwsError::Layout),
		};

		if let Some(statement) = statement {
			if !valid_statement(statement) {
				return Err(SiwsError::Statement);
			}

			input.statement = Some(statement.to_owned());
		}

		if let Some(fields) = fields {
			parse_fields(fields, &mut input)?;
		}

		Ok(Self {
			address: parsed,
			input,
		})
	}
}

/// Whether a paragraph is the fields: it starts with one's label.
fn is_fields(paragraph: &str) -> bool {
	LABELS.iter().any(|label| paragraph.starts_with(label))
		|| paragraph == "Resources:"
		|| paragraph.starts_with("Resources:\n")
}

/// A statement is one line of text that can't be mistaken for the fields.
pub(crate) fn valid_statement(statement: &str) -> bool {
	!statement.is_empty()
		&& statement.len() <= MAX_FIELD_LENGTH
		&& !statement.contains('\n')
		&& !statement.starts_with(' ')
		&& !statement.ends_with(' ')
		&& !is_fields(statement)
}

/// A host, with an optional port: letters, digits, dots, and hyphens.
pub(crate) fn valid_domain(domain: &str) -> bool {
	let (host, port) = match domain.rsplit_once(':') {
		Some((host, port)) => (host, Some(port)),
		None => (domain, None),
	};

	let host_ok = !host.is_empty()
		&& host.len() <= 253
		&& host
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
		&& !host.starts_with(['.', '-'])
		&& !host.ends_with(['.', '-']);
	let port_ok = port.is_none_or(|port| {
		!port.is_empty()
			&& port.len() <= 5
			&& port.bytes().all(|byte| byte.is_ascii_digit())
			&& !port.starts_with('0')
			&& port.parse::<u16>().is_ok()
	});
	host_ok && port_ok
}

fn parse_fields(fields: &str, input: &mut SiwsInput) -> Result<(), SiwsError> {
	let mut lines = fields.split('\n');
	let mut next_label = 0;

	for line in lines.by_ref() {
		if line == "Resources:" {
			input.resources = resources(lines)?;

			return Ok(());
		}

		let (index, value) = LABELS
			.iter()
			.enumerate()
			.skip(next_label)
			.find_map(|(index, label)| line.strip_prefix(label).map(|value| (index, value)))
			.ok_or(SiwsError::Field)?;

		if !valid_value(value) {
			return Err(SiwsError::Field);
		}

		next_label = index + 1;
		set_field(index, Some(value.to_owned()), input)?;
	}

	Ok(())
}

/// The resources listed after `Resources:`, one per `- ` line.
fn resources<'a>(lines: impl Iterator<Item = &'a str>) -> Result<Vec<String>, SiwsError> {
	let mut resources = Vec::new();

	for line in lines {
		let resource = line.strip_prefix("- ").ok_or(SiwsError::Field)?;

		if !valid_value(resource) {
			return Err(SiwsError::Field);
		}

		resources.push(resource.to_owned());
	}

	if resources.is_empty() {
		return Err(SiwsError::Field);
	}

	Ok(resources)
}

/// Stores one field, checking the ones with their own rule.
fn set_field(index: usize, value: Option<String>, input: &mut SiwsInput) -> Result<(), SiwsError> {
	match index {
		0 => input.uri = value,
		1 => {
			if value.as_deref() != Some("1") {
				return Err(SiwsError::Version);
			}

			input.version = value;
		}
		2 => input.chain_id = value,
		3 => {
			if !value.as_deref().is_some_and(valid_nonce) {
				return Err(SiwsError::Nonce);
			}

			input.nonce = value;
		}
		4 => input.issued_at = time(value)?,
		5 => input.expiration_time = time(value)?,
		6 => input.not_before = time(value)?,
		_ => input.request_id = value,
	}

	Ok(())
}

/// A field's value: not empty, not padded, and not too long.
fn valid_value(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= MAX_FIELD_LENGTH
		&& !value.starts_with(' ')
		&& !value.ends_with(' ')
}

/// EIP-4361's nonce: at least 8 letters and digits.
pub(crate) fn valid_nonce(nonce: &str) -> bool {
	nonce.len() >= 8 && nonce.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn time(value: Option<String>) -> Result<Option<String>, SiwsError> {
	match value {
		Some(text) if chrono::DateTime::parse_from_rfc3339(&text).is_ok() => Ok(Some(text)),
		_ => Err(SiwsError::Time),
	}
}

#[cfg(test)]
mod tests {
	use proptest::prelude::*;

	use super::*;

	const ADDRESS: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";

	fn address() -> Pubkey {
		Pubkey::parse(ADDRESS).unwrap()
	}

	fn input() -> SiwsInput {
		SiwsInput {
			domain: "notes.example".into(),
			address: None,
			statement: Some("Link this wallet to your account on Notes.".into()),
			uri: Some("https://notes.example".into()),
			version: Some("1".into()),
			chain_id: Some("mainnet".into()),
			nonce: Some("4WQ5WmPxpHdRqv8a3NhVF2".into()),
			issued_at: Some("2026-10-30T12:00:00Z".into()),
			expiration_time: Some("2026-10-30T12:05:00Z".into()),
			not_before: None,
			request_id: Some("link".into()),
			resources: Vec::new(),
		}
	}

	#[test]
	fn messages_render_as_the_wallet_standard_writes_them() {
		assert_eq!(
			input().message(&address()),
			"notes.example wants you to sign in with your Solana account:\n\
			 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM\n\
			 \n\
			 Link this wallet to your account on Notes.\n\
			 \n\
			 URI: https://notes.example\n\
			 Version: 1\n\
			 Chain ID: mainnet\n\
			 Nonce: 4WQ5WmPxpHdRqv8a3NhVF2\n\
			 Issued At: 2026-10-30T12:00:00Z\n\
			 Expiration Time: 2026-10-30T12:05:00Z\n\
			 Request ID: link"
		);
		let bare = SiwsInput {
			domain: "localhost:3000".into(),
			..SiwsInput::default()
		};
		assert_eq!(
			bare.message(&address()),
			format!("localhost:3000 wants you to sign in with your Solana account:\n{ADDRESS}")
		);
		let resources = SiwsInput {
			resources: vec!["https://notes.example/terms".into()],
			statement: None,
			..input()
		};
		let text = resources.message(&address());
		assert!(text.ends_with("Request ID: link\nResources:\n- https://notes.example/terms"));
		assert_eq!(SiwsMessage::parse(&text).unwrap().input, resources);
	}

	#[test]
	fn the_parser_reads_back_what_was_written() {
		let parsed = SiwsMessage::parse(&input().message(&address())).unwrap();
		assert_eq!(parsed.address, address());
		assert_eq!(parsed.input, input());

		let fields_only = SiwsInput {
			statement: None,
			..input()
		};
		let parsed = SiwsMessage::parse(&fields_only.message(&address())).unwrap();
		assert_eq!(parsed.input, fields_only);

		let statement_only = SiwsInput {
			domain: "notes.example".into(),
			statement: Some("Sign in.".into()),
			..SiwsInput::default()
		};
		let parsed = SiwsMessage::parse(&statement_only.message(&address())).unwrap();
		assert_eq!(parsed.input, statement_only);
	}

	#[test]
	fn anything_else_is_refused() {
		let good = input().message(&address());
		let cases: Vec<(String, SiwsError)> = vec![
			("x".repeat(MAX_MESSAGE_LENGTH + 1), SiwsError::TooLong),
			(good.replace('\n', "\r\n"), SiwsError::Character),
			(
				good.replacen("notes", "no\u{7}tes", 1),
				SiwsError::Character,
			),
			(
				good.replacen("wants you", "would like you", 1),
				SiwsError::Header,
			),
			(
				good.replacen("notes.example wants", " wants", 1),
				SiwsError::Header,
			),
			(
				good.replacen("notes.example wants", "notes example wants", 1),
				SiwsError::Header,
			),
			(
				good.replacen("notes.example wants", "notes.example:0080 wants", 1),
				SiwsError::Header,
			),
			(
				good.replacen("notes.example wants", "notes.example:99999 wants", 1),
				SiwsError::Header,
			),
			(
				good.replacen("notes.example wants", "https://notes.example wants", 1),
				SiwsError::Header,
			),
			(
				good.replacen(ADDRESS, &format!(" {ADDRESS}"), 1),
				SiwsError::Address,
			),
			(
				good.replacen(ADDRESS, "11111111111111111111111111111111111", 1),
				SiwsError::Address,
			),
			(good.replacen("\n\nURI", "\n\n\nURI", 1), SiwsError::Field),
			(
				good.replacen("\nVersion", "\n\nVersion", 1),
				SiwsError::Layout,
			),
			(
				good.replacen("Link this", "URI: this", 1),
				SiwsError::Statement,
			),
			(
				good.replacen("Version: 1", "Version: 2", 1),
				SiwsError::Version,
			),
			(
				good.replacen("Nonce: 4WQ5WmPxpHdRqv8a3NhVF2", "Nonce: short", 1),
				SiwsError::Nonce,
			),
			(
				good.replacen("Nonce: 4WQ5WmPxpHdRqv8a3NhVF2", "Nonce: has-dash-in-it", 1),
				SiwsError::Nonce,
			),
			(good.replacen("12:00:00Z", "noon", 1), SiwsError::Time),
			(
				good.replacen("Chain ID: mainnet\n", "", 1).replacen(
					"Version: 1",
					"Version: 1\nVersion: 1",
					1,
				),
				SiwsError::Field,
			),
			(
				good.replacen(
					"Version: 1\nChain ID: mainnet",
					"Chain ID: mainnet\nVersion: 1",
					1,
				),
				SiwsError::Field,
			),
			(
				good.replacen("Chain ID: mainnet", "Chain: mainnet", 1),
				SiwsError::Field,
			),
			(
				good.replacen("Chain ID: mainnet", "Chain ID: ", 1),
				SiwsError::Field,
			),
			(
				good.replacen("Chain ID: mainnet", "Chain ID:  mainnet", 1),
				SiwsError::Field,
			),
			(format!("{good}\n"), SiwsError::Field),
			(format!("{good}\nResources:"), SiwsError::Field),
			(format!("{good}\nResources:\n-nospace"), SiwsError::Field),
			(format!("{good}\n\nextra"), SiwsError::Layout),
			(ADDRESS.to_owned(), SiwsError::Header),
		];

		for (text, expected) in cases {
			assert_eq!(SiwsMessage::parse(&text), Err(expected), "{text:?}");
		}
	}

	#[test]
	fn each_refusal_says_which_rule() {
		let errors = [
			SiwsError::TooLong,
			SiwsError::Character,
			SiwsError::Header,
			SiwsError::Address,
			SiwsError::Layout,
			SiwsError::Statement,
			SiwsError::Field,
			SiwsError::Version,
			SiwsError::Nonce,
			SiwsError::Time,
		];
		let messages: std::collections::HashSet<String> =
			errors.iter().map(ToString::to_string).collect();
		assert_eq!(messages.len(), errors.len());
		let error: &dyn std::error::Error = &SiwsError::Nonce;
		assert!(error.to_string().contains("8 letters and digits"));
	}

	#[test]
	fn domains_are_hosts() {
		for good in [
			"notes.example",
			"localhost",
			"127.0.0.1:3000",
			"a-b.example:443",
		] {
			assert!(valid_domain(good), "{good}");
		}

		for bad in [
			"",
			":3000",
			"notes.example:",
			"-notes.example",
			"notes_example",
			"[::1]:3000",
			"notes.example:65536",
		] {
			assert!(!valid_domain(bad), "{bad}");
		}
	}

	#[test]
	fn the_input_travels_as_the_wallet_standard_names_it() {
		let json = serde_json::to_value(input()).unwrap();
		assert_eq!(json["chainId"], "mainnet");
		assert_eq!(json["issuedAt"], "2026-10-30T12:00:00Z");
		assert_eq!(json["expirationTime"], "2026-10-30T12:05:00Z");
		assert_eq!(json["requestId"], "link");
		assert!(json.get("address").is_none());
		assert!(json.get("resources").is_none());
		assert_eq!(serde_json::from_value::<SiwsInput>(json).unwrap(), input());
	}

	/// Inputs the renderer writes: every field optional, values free of
	/// newlines, padding, and the field labels.
	fn inputs() -> impl Strategy<Value = (SiwsInput, [u8; 32])> {
		let value = "[A-Za-z0-9][A-Za-z0-9 .:/_=-]{0,30}[A-Za-z0-9]";
		let time = prop::option::of((2000_i64..2100, 1_u32..13, 1_u32..29, 0_u32..24).prop_map(
			|(year, month, day, hour)| format!("{year}-{month:02}-{day:02}T{hour:02}:00:00Z"),
		));
		(
			(
				"[a-z][a-z0-9-]{0,20}[a-z0-9](\\.[a-z]{2,6})?(:[1-9][0-9]{0,3})?",
				prop::option::of("[A-Za-z0-9][A-Za-z0-9 ,.']{0,60}[A-Za-z0-9.]"),
				prop::option::of(value),
				prop::option::of(Just("1".to_owned())),
				prop::option::of(value),
				prop::option::of("[A-Za-z0-9]{8,40}"),
			),
			(
				time.clone(),
				time.clone(),
				time,
				prop::option::of(value),
				prop::collection::vec(value, 0..3),
				any::<[u8; 32]>(),
			),
		)
			.prop_filter_map(
				"statements can't look like fields",
				|(
					(domain, statement, uri, version, chain_id, nonce),
					(issued_at, expiration_time, not_before, request_id, resources, key),
				)| {
					if statement
						.as_deref()
						.is_some_and(|statement| !valid_statement(statement))
					{
						return None;
					}

					Some((
						SiwsInput {
							domain,
							address: None,
							statement,
							uri,
							version,
							chain_id,
							nonce,
							issued_at,
							expiration_time,
							not_before,
							request_id,
							resources,
						},
						key,
					))
				},
			)
	}

	proptest! {
		#![proptest_config(ProptestConfig::with_cases(1024))]

		#[test]
		fn every_rendered_message_parses_back((input, key) in inputs()) {
			let address = Pubkey::from_bytes(key);
			let text = input.message(&address);
			let parsed = SiwsMessage::parse(&text).unwrap();
			prop_assert_eq!(&parsed.address, &address);
			prop_assert_eq!(parsed.input, input);
		}

		#[test]
		fn whatever_parses_renders_back_byte_for_byte(text in "\\PC{0,300}") {
			if let Ok(parsed) = SiwsMessage::parse(&text) {
				prop_assert_eq!(parsed.input.message(&parsed.address), text);
			}
		}

		#[test]
		fn changing_a_valid_message_never_slips_past((input, key) in inputs(), position in any::<prop::sample::Index>(), insert in "[\\n :A-Za-z0-9\\-]") {
			// Fuzz: insert one character anywhere. The result either fails to
			// parse or parses to something that renders back to itself, never
			// to text that differs from what the parser read.
			let address = Pubkey::from_bytes(key);
			let mut text = input.message(&address);
			let at = position.index(text.len() + 1);
			let at = (0..=at).rev().find(|at| text.is_char_boundary(*at)).unwrap_or(0);
			text.insert_str(at, &insert);

			if let Ok(parsed) = SiwsMessage::parse(&text) {
				prop_assert_eq!(parsed.input.message(&parsed.address), text);
			}
		}

		#[test]
		fn mutated_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
			let text = String::from_utf8_lossy(&bytes);
			let _ = SiwsMessage::parse(&text);
		}
	}
}
