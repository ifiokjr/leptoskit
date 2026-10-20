//! Version-one binary HTTP request/response frames.
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

use crate::Header;
use crate::Method;
use crate::Request;
use crate::Response;

pub(crate) const MAX_HEADERS: usize = 128;
pub(crate) const MAX_URL: usize = 4096;

fn headers_out(out: &mut Encoder, headers: &[Header]) -> Result<(), Error> {
	if headers.len() > MAX_HEADERS {
		return Err(Error::TooLarge);
	}
	out.u32(headers.len() as u32);
	for header in headers {
		out.bytes(header.name.as_bytes())?;
		out.bytes(&header.value)?;
	}
	Ok(())
}
fn headers_in(input: &mut Decoder<'_>) -> Result<Vec<Header>, Error> {
	let count = input.u32()? as usize;
	if count > MAX_HEADERS {
		return Err(Error::TooLarge);
	}
	let mut headers = Vec::with_capacity(count);
	for _ in 0..count {
		headers.push(Header {
			name: input.string()?.to_owned(),
			value: input.bytes()?.to_vec(),
		});
	}
	Ok(headers)
}
fn url(input: &mut Decoder<'_>) -> Result<String, Error> {
	let value = input.string()?;
	if value.len() > MAX_URL {
		return Err(Error::TooLarge);
	}
	Ok(value.to_owned())
}

/// Encodes a typed guest request. The host still validates all scopes and
/// limits.
pub fn encode_request(request: &Request) -> Result<Bytes, Error> {
	if request.url.len() > MAX_URL {
		return Err(Error::TooLarge);
	}
	let mut out = Encoder::default();
	out.u32(match request.method {
		Method::Get => 0,
		Method::Head => 1,
		Method::Post => 2,
		Method::Put => 3,
		Method::Patch => 4,
		Method::Delete => 5,
		Method::Options => 6,
	});
	out.bytes(request.url.as_bytes())?;
	headers_out(&mut out, &request.headers)?;
	out.bytes(&request.body)?;
	Ok(out.finish())
}

/// Strictly decodes a request, rejecting malformed fields and trailing bytes.
pub fn decode_request(bytes: &[u8]) -> Result<Request, Error> {
	let mut input = Decoder::new(bytes);
	let method = match input.u32()? {
		0 => Method::Get,
		1 => Method::Head,
		2 => Method::Post,
		3 => Method::Put,
		4 => Method::Patch,
		5 => Method::Delete,
		6 => Method::Options,
		_ => return Err(Error::InvalidArgument),
	};
	let request = Request {
		method,
		url: url(&mut input)?,
		headers: headers_in(&mut input)?,
		body: input.bytes()?.to_vec(),
	};
	input.finish()?;
	Ok(request)
}

/// Encodes status, visible headers and binary body without a JSON envelope.
pub fn encode_response(response: &Response) -> Result<Bytes, Error> {
	if response.url.len() > MAX_URL {
		return Err(Error::TooLarge);
	}
	let mut out = Encoder::default();
	out.u32(u32::from(response.status));
	out.bytes(response.url.as_bytes())?;
	headers_out(&mut out, &response.headers)?;
	out.bytes(&response.body)?;
	Ok(out.finish())
}

/// Decodes a completed HTTP response.
pub fn decode_response(bytes: &[u8]) -> Result<Response, Error> {
	let mut input = Decoder::new(bytes);
	let status = u16::try_from(input.u32()?).map_err(|_| Error::InvalidArgument)?;
	if !(100..=599).contains(&status) {
		return Err(Error::InvalidArgument);
	}
	let response = Response {
		status,
		url: url(&mut input)?,
		headers: headers_in(&mut input)?,
		body: input.bytes()?.to_vec(),
	};
	input.finish()?;
	Ok(response)
}
