//! Reading and writing the path and query string of a route URL.

use std::fmt;

/// A URL's decoded path segments and query pairs: everything a route matches
/// against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteUrl {
	segments: Vec<String>,
	query: Vec<(String, String)>,
}

impl RouteUrl {
	/// Parses a path with an optional query string and fragment, such as
	/// `/notes/42?highlight=3#top`.
	///
	/// Empty path segments are skipped, so `/notes/` and `/notes` are the same
	/// URL. The fragment is ignored. In the query string, `+` decodes to a
	/// space.
	///
	/// # Errors
	///
	/// Returns [`UrlError`] when a percent escape is malformed or decodes to
	/// invalid UTF-8.
	pub fn parse(url: &str) -> Result<Self, UrlError> {
		let url = url.split_once('#').map_or(url, |(before, _)| before);
		let (path, query) = url.split_once('?').unwrap_or((url, ""));
		let segments = path
			.split('/')
			.filter(|segment| !segment.is_empty())
			.map(|segment| decode(segment, false))
			.collect::<Result<_, _>>()?;
		let query = query
			.split('&')
			.filter(|pair| !pair.is_empty())
			.map(|pair| {
				let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
				Ok((decode(key, true)?, decode(value, true)?))
			})
			.collect::<Result<_, UrlError>>()?;
		Ok(Self { segments, query })
	}

	/// The decoded path segments, without empty ones.
	pub fn segments(&self) -> &[String] {
		&self.segments
	}

	/// The decoded query pairs, in the order they appear.
	pub fn query(&self) -> &[(String, String)] {
		&self.query
	}
}

/// A URL that could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
	/// A `%` was not followed by two hex digits.
	#[error("malformed percent escape in `{0}`")]
	Escape(String),
	/// The percent escapes decoded to bytes that are not UTF-8.
	#[error("`{0}` does not decode to UTF-8")]
	Utf8(String),
}

/// A route's path and query string, ready to use as a link. It always starts
/// with `/`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Href(String);

impl Href {
	/// The href as a string slice.
	pub fn as_str(&self) -> &str {
		&self.0
	}

	pub(crate) fn from_parts(segments: &[String], query: &[(&str, String)]) -> Self {
		let mut href = String::new();
		for segment in segments {
			href.push('/');
			encode_into(&mut href, segment, PATH_SAFE);
		}
		if href.is_empty() {
			href.push('/');
		}
		for (index, (key, value)) in query.iter().enumerate() {
			href.push(if index == 0 { '?' } else { '&' });
			encode_into(&mut href, key, QUERY_SAFE);
			href.push('=');
			encode_into(&mut href, value, QUERY_SAFE);
		}
		Self(href)
	}
}

impl fmt::Display for Href {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl AsRef<str> for Href {
	fn as_ref(&self) -> &str {
		&self.0
	}
}

impl From<Href> for String {
	fn from(href: Href) -> Self {
		href.0
	}
}

/// Characters a path segment keeps as they are, besides ASCII letters and
/// digits.
const PATH_SAFE: &[u8] = b"-._~!$&'()*+,;=:@";

/// Characters a query key or value keeps as they are, besides ASCII letters
/// and digits. `&`, `=` and `+` are escaped, because they mean something in
/// a query string.
const QUERY_SAFE: &[u8] = b"-._~!$'()*,;:@/?";

fn encode_into(out: &mut String, text: &str, safe: &[u8]) {
	for byte in text.bytes() {
		if byte.is_ascii_alphanumeric() || safe.contains(&byte) {
			out.push(char::from(byte));
		} else {
			out.push('%');
			out.push(char::from(HEX[usize::from(byte >> 4)]));
			out.push(char::from(HEX[usize::from(byte & 0x0f)]));
		}
	}
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn decode(text: &str, plus_is_space: bool) -> Result<String, UrlError> {
	let escaped = text.contains('%') || (plus_is_space && text.contains('+'));
	if !escaped {
		return Ok(text.to_owned());
	}
	let bytes = text.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		match bytes[index] {
			b'%' => {
				let escape = bytes
					.get(index + 1..index + 3)
					.and_then(|hex| Some(hex_value(hex[0])? << 4 | hex_value(hex[1])?))
					.ok_or_else(|| UrlError::Escape(text.to_owned()))?;
				decoded.push(escape);
				index += 3;
			}
			b'+' if plus_is_space => {
				decoded.push(b' ');
				index += 1;
			}
			byte => {
				decoded.push(byte);
				index += 1;
			}
		}
	}
	String::from_utf8(decoded).map_err(|_| UrlError::Utf8(text.to_owned()))
}

fn hex_value(digit: u8) -> Option<u8> {
	match digit {
		b'0'..=b'9' => Some(digit - b'0'),
		b'a'..=b'f' => Some(digit - b'a' + 10),
		b'A'..=b'F' => Some(digit - b'A' + 10),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_segments_query_and_ignores_the_fragment() {
		let url = RouteUrl::parse("/notes//42/?highlight=3&tag=a&tag=b%20c#top").unwrap();
		assert_eq!(url.segments(), ["notes", "42"]);
		assert_eq!(
			url.query(),
			[
				("highlight".to_owned(), "3".to_owned()),
				("tag".to_owned(), "a".to_owned()),
				("tag".to_owned(), "b c".to_owned()),
			]
		);
	}

	#[test]
	fn decodes_plus_as_space_only_in_the_query() {
		let url = RouteUrl::parse("/a+b?q=a+b").unwrap();
		assert_eq!(url.segments(), ["a+b"]);
		assert_eq!(url.query(), [("q".to_owned(), "a b".to_owned())]);
	}

	#[test]
	fn the_root_has_no_segments() {
		assert!(RouteUrl::parse("/").unwrap().segments().is_empty());
		assert!(RouteUrl::parse("").unwrap().segments().is_empty());
	}

	#[test]
	fn rejects_malformed_escapes_and_invalid_utf8() {
		assert_eq!(
			RouteUrl::parse("/%zz"),
			Err(UrlError::Escape("%zz".to_owned()))
		);
		assert_eq!(
			RouteUrl::parse("/%4"),
			Err(UrlError::Escape("%4".to_owned()))
		);
		assert_eq!(
			RouteUrl::parse("/%ff"),
			Err(UrlError::Utf8("%ff".to_owned()))
		);
	}

	#[test]
	fn hrefs_escape_what_would_change_their_meaning() {
		let href = Href::from_parts(
			&["a b".to_owned(), "x/y".to_owned(), "ünï".to_owned()],
			&[("q", "1&2=3+4".to_owned())],
		);
		assert_eq!(href.as_str(), "/a%20b/x%2Fy/%C3%BCn%C3%AF?q=1%262%3D3%2B4");
		let url = RouteUrl::parse(href.as_str()).unwrap();
		assert_eq!(url.segments(), ["a b", "x/y", "ünï"]);
		assert_eq!(url.query(), [("q".to_owned(), "1&2=3+4".to_owned())]);
	}

	#[test]
	fn an_href_reads_as_a_string() {
		let href = Href::from_parts(&["notes".to_owned()], &[]);
		assert_eq!(href.to_string(), "/notes");
		assert_eq!(href.as_ref(), "/notes");
		assert_eq!(String::from(href), "/notes");
	}

	#[test]
	fn an_empty_href_is_the_root() {
		assert_eq!(Href::from_parts(&[], &[]).as_str(), "/");
		assert_eq!(
			Href::from_parts(&[], &[("a", "b".to_owned())]).as_str(),
			"/?a=b"
		);
	}
}
