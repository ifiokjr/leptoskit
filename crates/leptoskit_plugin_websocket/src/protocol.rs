//! Strict version-one binary WebSocket frames.
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

use crate::Message;

/// Host-assigned connection identifier, never reused by an installed plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionId(pub u32);

/// Encodes a scoped connection request.
pub fn open(url: &str, protocols: &[String]) -> Result<Bytes, Error> {
	if url.len() > 4096 || protocols.len() > 32 {
		return Err(Error::TooLarge);
	}
	let mut out = Encoder::default();
	out.bytes(url.as_bytes())?;
	out.u32(protocols.len() as u32);
	for value in protocols {
		if value.len() > 256 {
			return Err(Error::TooLarge);
		}
		out.bytes(value.as_bytes())?;
	}
	Ok(out.finish())
}
/// Decodes a connection request before network access.
pub fn decode_open(bytes: &[u8]) -> Result<(String, Vec<String>), Error> {
	let mut input = Decoder::new(bytes);
	let url = input.string()?;
	if url.len() > 4096 {
		return Err(Error::TooLarge);
	}
	let count = input.u32()?;
	if count > 32 {
		return Err(Error::TooLarge);
	}
	let mut protocols = Vec::with_capacity(count as usize);
	for _ in 0..count {
		let value = input.string()?;
		if value.len() > 256 {
			return Err(Error::TooLarge);
		}
		protocols.push(value.to_owned());
	}
	input.finish()?;
	Ok((url.to_owned(), protocols))
}
/// Encodes a connection identifier.
pub fn id(id: ConnectionId) -> Bytes {
	Bytes::copy_from_slice(&id.0.to_le_bytes())
}
/// Decodes a connection identifier.
pub fn decode_id(bytes: &[u8]) -> Result<ConnectionId, Error> {
	let mut input = Decoder::new(bytes);
	let id = input.u32()?;
	input.finish()?;
	if id == 0 {
		return Err(Error::InvalidArgument);
	}
	Ok(ConnectionId(id))
}
/// Encodes text, binary, or the end of the receive stream.
pub fn message(value: Option<&Message>) -> Result<Bytes, Error> {
	let mut out = Encoder::default();
	match value {
		None => out.u32(0),
		Some(Message::Text(text)) => {
			out.u32(1);
			out.bytes(text.as_bytes())?;
		}
		Some(Message::Binary(bytes)) => {
			out.u32(2);
			out.bytes(bytes)?;
		}
	}
	Ok(out.finish())
}
/// Decodes text, binary, or orderly close.
pub fn decode_message(bytes: &[u8]) -> Result<Option<Message>, Error> {
	let mut input = Decoder::new(bytes);
	let value = match input.u32()? {
		0 => None,
		1 => Some(Message::Text(input.string()?.to_owned())),
		2 => Some(Message::Binary(input.bytes()?.to_vec())),
		_ => return Err(Error::InvalidArgument),
	};
	input.finish()?;
	Ok(value)
}
/// Encodes a send request.
pub fn send(connection: ConnectionId, value: &Message) -> Result<Bytes, Error> {
	let mut out = Encoder::default();
	out.u32(connection.0);
	out.bytes(&message(Some(value))?)?;
	Ok(out.finish())
}
/// Decodes a send request.
pub fn decode_send(bytes: &[u8]) -> Result<(ConnectionId, Message), Error> {
	let mut input = Decoder::new(bytes);
	let id = ConnectionId(input.u32()?);
	let value = decode_message(input.bytes()?)?.ok_or(Error::InvalidArgument)?;
	input.finish()?;
	Ok((id, value))
}
/// Encodes an explicit close request.
pub fn close(connection: ConnectionId, code: u16, reason: &str) -> Result<Bytes, Error> {
	let mut out = Encoder::default();
	out.u32(connection.0);
	out.u32(u32::from(code));
	out.bytes(reason.as_bytes())?;
	Ok(out.finish())
}
/// Decodes an explicit close request.
pub fn decode_close(bytes: &[u8]) -> Result<(ConnectionId, u16, String), Error> {
	let mut input = Decoder::new(bytes);
	let id = ConnectionId(input.u32()?);
	let code = u16::try_from(input.u32()?).map_err(|_| Error::InvalidArgument)?;
	let reason = input.string()?.to_owned();
	input.finish()?;
	Ok((id, code, reason))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn open_requests_bound_urls_and_subprotocols() {
		let protocols = vec!["chat".to_owned(), "v2".to_owned()];
		let encoded = open("wss://example.test/socket", &protocols).unwrap();
		assert_eq!(
			decode_open(&encoded).unwrap(),
			("wss://example.test/socket".to_owned(), protocols)
		);
		let long_url = "a".repeat(4097);
		assert_eq!(open(&long_url, &[]).unwrap_err(), Error::TooLarge);
		assert_eq!(
			open("wss://example.test", &vec![String::new(); 33]).unwrap_err(),
			Error::TooLarge
		);
		assert_eq!(
			open("wss://example.test", &["p".repeat(257)]).unwrap_err(),
			Error::TooLarge
		);
		let frame = |url: &str, count: u32, protocol: &str| {
			let mut out = Encoder::default();
			out.bytes(url.as_bytes()).unwrap();
			out.u32(count);
			if count == 1 {
				out.bytes(protocol.as_bytes()).unwrap();
			}
			out.finish()
		};
		assert_eq!(
			decode_open(&frame(&long_url, 0, "")).unwrap_err(),
			Error::TooLarge
		);
		assert_eq!(
			decode_open(&frame("wss://example.test", 33, "")).unwrap_err(),
			Error::TooLarge
		);
		assert_eq!(
			decode_open(&frame("wss://example.test", 1, &"p".repeat(257))).unwrap_err(),
			Error::TooLarge
		);
	}
	#[test]
	fn messages_ids_sends_and_closes_are_exact() {
		for value in [
			None,
			Some(Message::Text("hello".into())),
			Some(Message::Binary(vec![0, 255])),
		] {
			assert_eq!(
				decode_message(&message(value.as_ref()).unwrap()).unwrap(),
				value
			);
		}
		let mut unknown = message(None).unwrap().to_vec();
		unknown[0] = 3;
		assert_eq!(
			decode_message(&unknown).unwrap_err(),
			Error::InvalidArgument
		);
		assert_eq!(decode_id(&id(ConnectionId(9))).unwrap(), ConnectionId(9));
		assert_eq!(decode_id(&[0; 4]).unwrap_err(), Error::InvalidArgument);
		let text = Message::Text("ping".into());
		assert_eq!(
			decode_send(&send(ConnectionId(3), &text).unwrap()).unwrap(),
			(ConnectionId(3), text)
		);
		let mut end_of_stream = Encoder::default();
		end_of_stream.u32(3);
		end_of_stream.bytes(&message(None).unwrap()).unwrap();
		assert_eq!(
			decode_send(&end_of_stream.finish()).unwrap_err(),
			Error::InvalidArgument
		);
		assert_eq!(
			decode_close(&close(ConnectionId(4), 1000, "done").unwrap()).unwrap(),
			(ConnectionId(4), 1000, "done".to_owned())
		);
		let mut wide = close(ConnectionId(4), 1000, "").unwrap().to_vec();
		wide[4..8].copy_from_slice(&65_536_u32.to_le_bytes());
		assert_eq!(decode_close(&wide).unwrap_err(), Error::InvalidArgument);
	}
}
