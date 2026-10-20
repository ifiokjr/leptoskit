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
