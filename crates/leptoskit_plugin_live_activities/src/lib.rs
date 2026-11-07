//! <!-- {=p_live_activitiesSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_live_activities` as a direct dependency and select the
//! `live-activities` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Windows, Linux, and terminal
//! hosts currently omit runtime plugins; use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::LiveActivities` rather than resolving the plugin on every call. Only
//! iOS implements `ActivityKit`; other platforms return `Unavailable`. Starting
//! an activity does not prove Lock Screen or Dynamic Island UI appeared: the
//! app must ship a Widget Extension that renders
//! `LeptosKitLiveActivityAttributes`.
//!
//! <!-- {/p_live_activitiesSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "live-activities";
/// Empty payload; whether `ActivityKit` currently allows Live Activities.
pub const ARE_ENABLED: MethodId = MethodId(0);
/// Empty payload; whether frequent push updates are currently allowed.
pub const FREQUENT_UPDATES_ENABLED: MethodId = MethodId(1);
/// Start request; echoes the activity identifier on success.
pub const START: MethodId = MethodId(2);
/// Update request; empty reply.
pub const UPDATE: MethodId = MethodId(3);
/// End request; empty reply.
pub const END: MethodId = MethodId(4);
/// Activity id; waits for the next `ActivityKit` push token for that activity.
pub const WAIT_PUSH_TOKEN: MethodId = MethodId(5);
/// Empty payload; identifiers of activities this plugin currently tracks.
pub const LIST: MethodId = MethodId(6);
/// Independently grantable operations.
pub const METHODS: &[MethodId] = &[
	ARE_ENABLED,
	FREQUENT_UPDATES_ENABLED,
	START,
	UPDATE,
	END,
	WAIT_PUSH_TOKEN,
	LIST,
];

const MAX_ID_LEN: usize = 64;
const MAX_TITLE_LEN: usize = 128;
const MAX_FIELDS: usize = 16;
const MAX_FIELD_KEY_LEN: usize = 64;
const MAX_FIELD_VALUE_LEN: usize = 256;
const MAX_PUSH_TOKEN_LEN: usize = 512;
const MAX_START_BYTES: usize = 4096;
const MAX_UPDATE_BYTES: usize = 4096;
const MAX_END_BYTES: usize = 4096;
const MAX_AFTER_SECONDS: u32 = 86_400;

/// Application-chosen activity identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier(String);

impl Identifier {
	/// Accepts 1..=64 ASCII letters, digits, hyphens, periods or underscores.
	pub fn new(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_id(&value)?;
		Ok(Self(value))
	}

	/// Borrows the identifier.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	fn encode_into(&self, output: &mut Encoder) -> Result<(), Error> {
		output.bytes(self.0.as_bytes())
	}

	fn decode(input: &mut Decoder<'_>) -> Result<Self, Error> {
		Self::new(input.string()?)
	}
}

fn validate_id(value: &str) -> Result<(), Error> {
	if value.is_empty()
		|| value.len() > MAX_ID_LEN
		|| !value
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_text(value: &str, maximum: usize, required: bool) -> Result<(), Error> {
	if value.len() > maximum
		|| (required && value.trim().is_empty())
		|| value.chars().any(char::is_control)
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

/// Ordered string fields that populate the Live Activity content state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentState {
	fields: Vec<(String, String)>,
}

impl ContentState {
	/// Builds a content state from key/value pairs. Keys follow identifier
	/// rules; values are plain text up to 256 UTF-8 bytes.
	pub fn new(
		fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
	) -> Result<Self, Error> {
		let mut out = Vec::new();
		for (key, value) in fields {
			let key = key.into();
			let value = value.into();
			validate_id_len(&key, MAX_FIELD_KEY_LEN)?;
			validate_text(&value, MAX_FIELD_VALUE_LEN, false)?;
			out.push((key, value));
			if out.len() > MAX_FIELDS {
				return Err(Error::InvalidArgument);
			}
		}
		Ok(Self { fields: out })
	}

	/// Borrows the ordered fields.
	#[must_use]
	pub fn fields(&self) -> &[(String, String)] {
		&self.fields
	}

	fn encode_into(&self, output: &mut Encoder) -> Result<(), Error> {
		output.u32(u32::try_from(self.fields.len()).map_err(|_| Error::TooLarge)?);
		for (key, value) in &self.fields {
			output.bytes(key.as_bytes())?;
			output.bytes(value.as_bytes())?;
		}
		Ok(())
	}

	fn decode(input: &mut Decoder<'_>) -> Result<Self, Error> {
		let count = input.u32()? as usize;
		if count > MAX_FIELDS {
			return Err(Error::InvalidArgument);
		}
		let mut fields = Vec::with_capacity(count);
		for _ in 0..count {
			let key = input.string()?;
			let value = input.string()?;
			validate_id_len(key, MAX_FIELD_KEY_LEN)?;
			validate_text(value, MAX_FIELD_VALUE_LEN, false)?;
			fields.push((key.to_owned(), value.to_owned()));
		}
		Ok(Self { fields })
	}
}

fn validate_id_len(value: &str, maximum: usize) -> Result<(), Error> {
	if value.is_empty()
		|| value.len() > maximum
		|| !value
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

/// How long the system keeps the ended activity visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissalPolicy {
	/// System default dismissal timing after end.
	Default,
	/// Remove the activity immediately.
	Immediate,
	/// Keep it visible for at most this many seconds after end (1..=86400).
	After { seconds: u32 },
}

impl DismissalPolicy {
	fn encode(self, output: &mut Encoder) {
		match self {
			Self::Default => {
				output.u32(0);
				output.u32(0);
			}
			Self::Immediate => {
				output.u32(1);
				output.u32(0);
			}
			Self::After { seconds } => {
				output.u32(2);
				output.u32(seconds);
			}
		}
	}

	fn decode(input: &mut Decoder<'_>) -> Result<Self, Error> {
		match input.u32()? {
			0 => {
				if input.u32()? != 0 {
					return Err(Error::InvalidArgument);
				}
				Ok(Self::Default)
			}
			1 => {
				if input.u32()? != 0 {
					return Err(Error::InvalidArgument);
				}
				Ok(Self::Immediate)
			}
			2 => {
				let seconds = input.u32()?;
				if !(1..=MAX_AFTER_SECONDS).contains(&seconds) {
					return Err(Error::InvalidArgument);
				}
				Ok(Self::After { seconds })
			}
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Validated start request for a Live Activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
	id: Identifier,
	kind: Identifier,
	title: String,
	content: ContentState,
	push_updates: bool,
}

impl StartRequest {
	/// Constructs a bounded start request.
	///
	/// `kind` groups activities for the Widget Extension. `push_updates`
	/// asks `ActivityKit` for a per-activity push token when the app has Push
	/// Notifications configured.
	pub fn new(
		id: Identifier,
		kind: Identifier,
		title: impl Into<String>,
		content: ContentState,
		push_updates: bool,
	) -> Result<Self, Error> {
		let title = title.into();
		validate_text(&title, MAX_TITLE_LEN, true)?;
		Ok(Self {
			id,
			kind,
			title,
			content,
			push_updates,
		})
	}

	/// Application-chosen replacement identifier.
	#[must_use]
	pub fn id(&self) -> &Identifier {
		&self.id
	}

	/// Widget Extension grouping key.
	#[must_use]
	pub fn kind(&self) -> &Identifier {
		&self.kind
	}

	/// Static title stored in activity attributes.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Initial content-state fields.
	#[must_use]
	pub fn content(&self) -> &ContentState {
		&self.content
	}

	/// Whether `ActivityKit` should mint a push token for remote updates.
	#[must_use]
	pub fn push_updates(&self) -> bool {
		self.push_updates
	}

	/// Encodes the start control message.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(64);
		self.id.encode_into(&mut output)?;
		self.kind.encode_into(&mut output)?;
		output.bytes(self.title.as_bytes())?;
		output.u32(u32::from(self.push_updates));
		self.content.encode_into(&mut output)?;
		Ok(output.finish())
	}

	/// Validates a complete start message.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_START_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let id = Identifier::decode(&mut input)?;
		let kind = Identifier::decode(&mut input)?;
		let title = input.string()?;
		let push_updates = match input.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};
		let content = ContentState::decode(&mut input)?;
		input.finish()?;
		Self::new(id, kind, title, content, push_updates)
	}
}

/// Validated content update for a running activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
	id: Identifier,
	content: ContentState,
}

impl UpdateRequest {
	/// Builds an update for an existing activity identifier.
	pub fn new(id: Identifier, content: ContentState) -> Self {
		Self { id, content }
	}

	/// Target activity.
	#[must_use]
	pub fn id(&self) -> &Identifier {
		&self.id
	}

	/// Replacement content state.
	#[must_use]
	pub fn content(&self) -> &ContentState {
		&self.content
	}

	/// Encodes the update control message.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(32);
		self.id.encode_into(&mut output)?;
		self.content.encode_into(&mut output)?;
		Ok(output.finish())
	}

	/// Validates a complete update message.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_UPDATE_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let id = Identifier::decode(&mut input)?;
		let content = ContentState::decode(&mut input)?;
		input.finish()?;
		Ok(Self::new(id, content))
	}
}

/// Validated end request, optionally with a final content state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndRequest {
	id: Identifier,
	dismissal: DismissalPolicy,
	final_content: Option<ContentState>,
}

impl EndRequest {
	/// Builds an end request.
	pub fn new(
		id: Identifier,
		dismissal: DismissalPolicy,
		final_content: Option<ContentState>,
	) -> Self {
		Self {
			id,
			dismissal,
			final_content,
		}
	}

	/// Target activity.
	#[must_use]
	pub fn id(&self) -> &Identifier {
		&self.id
	}

	/// Dismissal timing after end.
	#[must_use]
	pub fn dismissal(&self) -> DismissalPolicy {
		self.dismissal
	}

	/// Optional final content shown until dismissal.
	#[must_use]
	pub fn final_content(&self) -> Option<&ContentState> {
		self.final_content.as_ref()
	}

	/// Encodes the end control message.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(32);
		self.id.encode_into(&mut output)?;
		self.dismissal.encode(&mut output);
		match &self.final_content {
			None => output.u32(0),
			Some(content) => {
				output.u32(1);
				content.encode_into(&mut output)?;
			}
		}
		Ok(output.finish())
	}

	/// Validates a complete end message.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_END_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let id = Identifier::decode(&mut input)?;
		let dismissal = DismissalPolicy::decode(&mut input)?;
		let final_content = match input.u32()? {
			0 => None,
			1 => Some(ContentState::decode(&mut input)?),
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Ok(Self::new(id, dismissal, final_content))
	}
}

/// `ActivityKit` push token bytes for remote Live Activity updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushToken(Vec<u8>);

impl PushToken {
	/// Accepts a non-empty token up to 512 bytes.
	pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, Error> {
		let bytes = bytes.into();
		if bytes.is_empty() || bytes.len() > MAX_PUSH_TOKEN_LEN {
			Err(Error::InvalidArgument)
		} else {
			Ok(Self(bytes))
		}
	}

	/// Borrows the raw token bytes.
	#[must_use]
	pub fn as_bytes(&self) -> &[u8] {
		&self.0
	}

	/// Lowercase hex encoding for logging or APNs tooling.
	#[must_use]
	pub fn to_hex(&self) -> String {
		let mut hex = String::with_capacity(self.0.len() * 2);
		for byte in &self.0 {
			use std::fmt::Write as _;
			let _ = write!(hex, "{byte:02x}");
		}
		hex
	}

	/// Encodes a length-prefixed token reply.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.0.len());
		output.bytes(&self.0)?;
		Ok(output.finish())
	}

	/// Validates a complete push-token reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(payload);
		let token = Self::new(input.bytes()?.to_vec())?;
		input.finish()?;
		Ok(token)
	}
}

/// Decodes a little-endian boolean word.
pub fn decode_flag(bytes: &[u8]) -> Result<bool, Error> {
	match u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?) {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Encodes a little-endian boolean word.
#[must_use]
pub fn encode_flag(value: bool) -> Bytes {
	Bytes::from(u32::from(value).to_le_bytes().to_vec())
}

/// Decodes a list of activity identifiers.
pub fn decode_list(payload: &[u8]) -> Result<Vec<Identifier>, Error> {
	let mut input = Decoder::new(payload);
	let count = input.u32()? as usize;
	if count > 64 {
		return Err(Error::InvalidArgument);
	}
	let mut ids = Vec::with_capacity(count);
	for _ in 0..count {
		ids.push(Identifier::decode(&mut input)?);
	}
	input.finish()?;
	Ok(ids)
}

/// Encodes a list of activity identifiers.
pub fn encode_list(ids: &[Identifier]) -> Result<Bytes, Error> {
	if ids.len() > 64 {
		return Err(Error::InvalidArgument);
	}
	let mut output = Encoder::with_capacity(4);
	output.u32(u32::try_from(ids.len()).map_err(|_| Error::TooLarge)?);
	for id in ids {
		id.encode_into(&mut output)?;
	}
	Ok(output.finish())
}

/// Validates empty-payload methods before a platform call.
pub fn validate_empty(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	if !matches!(method, ARE_ENABLED | FREQUENT_UPDATES_ENABLED | LIST) {
		return Err(Error::Unavailable);
	}
	if payload.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached native Live Activities client.
	#[derive(Debug, Clone, Copy)]
	pub struct LiveActivities(leptoskit_bridge::guest::Client);

	impl LiveActivities {
		/// Resolves the installed host plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Whether `ActivityKit` currently allows Live Activities.
		pub async fn are_enabled(self) -> Result<bool, Error> {
			decode_flag(&self.0.call(ARE_ENABLED, &[])?.await?)
		}

		/// Whether frequent push updates are currently allowed.
		pub async fn frequent_updates_enabled(self) -> Result<bool, Error> {
			decode_flag(&self.0.call(FREQUENT_UPDATES_ENABLED, &[])?.await?)
		}

		/// Starts a Live Activity. Acceptance does not prove UI appeared.
		pub async fn start(self, request: &StartRequest) -> Result<Identifier, Error> {
			let bytes = self.0.call(START, &request.encode()?)?.await?;
			let mut input = Decoder::new(&bytes);
			let id = Identifier::decode(&mut input)?;
			input.finish()?;
			Ok(id)
		}

		/// Replaces the content state of a running activity.
		pub async fn update(self, request: &UpdateRequest) -> Result<(), Error> {
			self.empty(UPDATE, &request.encode()?).await
		}

		/// Ends a running activity with an optional final state.
		pub async fn end(self, request: &EndRequest) -> Result<(), Error> {
			self.empty(END, &request.encode()?).await
		}

		/// Waits for the next `ActivityKit` push token for this activity.
		pub async fn wait_push_token(self, id: &Identifier) -> Result<PushToken, Error> {
			let mut output = Encoder::with_capacity(4 + id.as_str().len());
			id.encode_into(&mut output)?;
			PushToken::decode(&self.0.call(WAIT_PUSH_TOKEN, &output.finish())?.await?)
		}

		/// Identifiers this plugin currently tracks.
		pub async fn list(self) -> Result<Vec<Identifier>, Error> {
			decode_list(&self.0.call(LIST, &[])?.await?)
		}

		async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
			if self.0.call(method, payload)?.await?.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

/// Browsers have no `ActivityKit`. Every operation is unavailable.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::future::Ready;
	use std::future::ready;

	use super::*;

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn are_enabled() -> Ready<Result<bool, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn frequent_updates_enabled() -> Ready<Result<bool, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn start(_request: &StartRequest) -> Ready<Result<Identifier, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn update(_request: &UpdateRequest) -> Ready<Result<(), Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn end(_request: &EndRequest) -> Ready<Result<(), Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn wait_push_token(_id: &Identifier) -> Ready<Result<PushToken, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable outside iOS `ActivityKit`.
	pub fn list() -> Ready<Result<Vec<Identifier>, Error>> {
		ready(Err(Error::Unavailable))
	}
}

/// Explicit unavailable handler for hosts without an iOS `ActivityKit` adapter.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			ARE_ENABLED | FREQUENT_UPDATES_ENABLED | LIST => validate_empty(method, &payload)?,
			START => {
				StartRequest::decode(&payload)?;
			}
			UPDATE => {
				UpdateRequest::decode(&payload)?;
			}
			END => {
				EndRequest::decode(&payload)?;
			}
			WAIT_PUSH_TOKEN => {
				let mut input = Decoder::new(&payload);
				Identifier::decode(&mut input)?;
				input.finish()?;
			}
			_ => return Err(Error::Unavailable),
		}
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin as _;

	use super::*;

	#[test]
	fn start_update_end_roundtrip_and_reject_bad_input() {
		let content = ContentState::new([("status", "boarding"), ("gate", "A12")]).unwrap();
		let start = StartRequest::new(
			Identifier::new("flight-42").unwrap(),
			Identifier::new("flight").unwrap(),
			"SFO → JFK",
			content.clone(),
			true,
		)
		.unwrap();
		let encoded = start.encode().unwrap();
		assert_eq!(StartRequest::decode(&encoded).unwrap(), start);
		for length in 0..encoded.len() {
			assert!(StartRequest::decode(&encoded[..length]).is_err());
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(StartRequest::decode(&trailing).is_err());

		let update = UpdateRequest::new(start.id().clone(), content);
		assert_eq!(
			UpdateRequest::decode(&update.encode().unwrap()).unwrap(),
			update
		);

		let end = EndRequest::new(
			start.id().clone(),
			DismissalPolicy::After { seconds: 30 },
			Some(ContentState::new([("status", "landed")]).unwrap()),
		);
		assert_eq!(EndRequest::decode(&end.encode().unwrap()).unwrap(), end);
		assert!(
			EndRequest::decode(
				&EndRequest::new(
					start.id().clone(),
					DismissalPolicy::After { seconds: 0 },
					None
				)
				.encode()
				.unwrap()
			)
			.is_err()
		);

		assert!(Identifier::new("").is_err());
		assert!(Identifier::new("bad id").is_err());
		assert!(
			StartRequest::new(
				Identifier::new("a").unwrap(),
				Identifier::new("b").unwrap(),
				" ",
				ContentState::default(),
				false
			)
			.is_err()
		);
		assert!(ContentState::new([("x".repeat(65), "v")]).is_err());
	}

	#[test]
	fn push_token_list_and_flags_are_strict() {
		let token = PushToken::new(vec![0xab, 0xcd]).unwrap();
		assert_eq!(token.to_hex(), "abcd");
		assert_eq!(PushToken::decode(&token.encode().unwrap()).unwrap(), token);
		assert!(PushToken::new(Vec::new()).is_err());
		assert!(PushToken::new(vec![0; 513]).is_err());

		assert_eq!(decode_flag(&[1, 0, 0, 0]), Ok(true));
		assert_eq!(decode_flag(&[0, 0, 0, 0]), Ok(false));
		assert!(decode_flag(&[2, 0, 0, 0]).is_err());

		let ids = [
			Identifier::new("one").unwrap(),
			Identifier::new("two").unwrap(),
		];
		assert_eq!(decode_list(&encode_list(&ids).unwrap()).unwrap(), ids);
	}

	#[test]
	fn unsupported_validates_then_refuses() {
		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(ARE_ENABLED, Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(ARE_ENABLED, Bytes::from(vec![0])),
			Err(Error::InvalidArgument)
		);
		let start = StartRequest::new(
			Identifier::new("ride-1").unwrap(),
			Identifier::new("ride").unwrap(),
			"Driver en route",
			ContentState::new([("eta", "4")]).unwrap(),
			false,
		)
		.unwrap();
		assert_eq!(
			plugin.invoke(START, start.encode().unwrap()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(MethodId(99), Bytes::new()),
			Err(Error::Unavailable)
		);
	}
}
