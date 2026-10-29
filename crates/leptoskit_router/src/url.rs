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
	/// Parsing never fails: it reads a URL the way a browser does, so a link
	/// someone typed or shared always reaches a route. Empty path segments
	/// are skipped, so `/notes/` and `/notes` are the same URL, and `.` and
	/// `..` segments are resolved, so `/notes/../map` is `/map`. A `%` that
	/// does not start a two-digit escape stays as it is, so `?near=100%`
	/// reads `100%`, and escapes that do not decode to UTF-8 become U+FFFD.
	/// In the query string, `+` decodes to a space. The fragment is ignored.
	pub fn parse(url: &str) -> Self {
		let url = url.split_once('#').map_or(url, |(before, _)| before);
		let (path, query) = url.split_once('?').unwrap_or((url, ""));
		let mut segments: Vec<String> = Vec::new();
		for segment in path.split('/').filter(|segment| !segment.is_empty()) {
			// Decoding first also resolves `%2e` and `%2e%2e`, which browsers
			// treat as dot segments too.
			let segment = decode(segment, false);
			match segment.as_str() {
				"." => {}
				".." => {
					segments.pop();
				}
				_ => segments.push(segment),
			}
		}
		let query = query
			.split('&')
			.filter(|pair| !pair.is_empty())
			.map(|pair| {
				let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
				(decode(key, true), decode(value, true))
			})
			.collect();
		Self { segments, query }
	}

	/// The decoded path segments. None is empty, `.` or `..`.
	pub fn segments(&self) -> &[String] {
		&self.segments
	}

	/// The decoded query pairs, in the order they appear.
	pub fn query(&self) -> &[(String, String)] {
		&self.query
	}
}

/// A route's path and query string, ready to use as a link. It always starts
/// with `/`, never with `//`, so it never leaves the app's origin.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Href(String);

/// Whether a URL path can hold `value` as one segment. Browsers drop empty
/// segments and resolve `.` and `..`, escaped or not, so no URL can carry
/// those values.
pub(crate) fn is_segment(value: &str) -> bool {
	!matches!(value, "" | "." | "..")
}

impl Href {
	/// The href as a string slice.
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// An href from path segments that each pass [`is_segment`], and query
	/// pairs.
	pub(crate) fn from_parts(segments: &[String], query: &[(&str, String)]) -> Self {
		let mut href = String::new();
		for segment in segments {
			debug_assert!(is_segment(segment), "`{segment}` is not a path segment");
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

/// Percent-decodes `text` the way browsers do: a `%` not followed by two hex
/// digits stays as it is, and bytes that are not UTF-8 become U+FFFD.
fn decode(text: &str, plus_is_space: bool) -> String {
	let escaped = text.contains('%') || (plus_is_space && text.contains('+'));
	if !escaped {
		return text.to_owned();
	}
	let bytes = text.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		let escape = bytes
			.get(index..index + 3)
			.filter(|escape| escape[0] == b'%')
			.and_then(|escape| Some(hex_value(escape[1])? << 4 | hex_value(escape[2])?));
		match (bytes[index], escape) {
			(_, Some(byte)) => {
				decoded.push(byte);
				index += 3;
			}
			(b'+', None) if plus_is_space => {
				decoded.push(b' ');
				index += 1;
			}
			(byte, None) => {
				decoded.push(byte);
				index += 1;
			}
		}
	}
	String::from_utf8(decoded)
		.unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
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

	fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
		pairs
			.iter()
			.map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
			.collect()
	}

	#[test]
	fn parses_segments_query_and_ignores_the_fragment() {
		let url = RouteUrl::parse("/notes//42/?highlight=3&tag=a&tag=b%20c#top");
		assert_eq!(url.segments(), ["notes", "42"]);
		assert_eq!(
			url.query(),
			query(&[("highlight", "3"), ("tag", "a"), ("tag", "b c")])
		);
	}

	#[test]
	fn decodes_plus_as_space_only_in_the_query() {
		let url = RouteUrl::parse("/a+b?q=a+b");
		assert_eq!(url.segments(), ["a+b"]);
		assert_eq!(url.query(), query(&[("q", "a b")]));
	}

	#[test]
	fn the_root_has_no_segments() {
		assert!(RouteUrl::parse("/").segments().is_empty());
		assert!(RouteUrl::parse("").segments().is_empty());
	}

	#[test]
	fn keeps_malformed_escapes_and_replaces_invalid_utf8_like_a_browser() {
		let url = RouteUrl::parse("/%zz/%4/%ff/caf%C3%A9?near=100%&utm=%zz&q=%E2%82");
		assert_eq!(url.segments(), ["%zz", "%4", "\u{FFFD}", "café"]);
		assert_eq!(
			url.query(),
			query(&[("near", "100%"), ("utm", "%zz"), ("q", "\u{FFFD}")])
		);
	}

	#[test]
	fn resolves_dot_segments_like_a_browser() {
		assert_eq!(RouteUrl::parse("/notes/./42").segments(), ["notes", "42"]);
		assert_eq!(RouteUrl::parse("/notes/../map").segments(), ["map"]);
		assert_eq!(RouteUrl::parse("/a/%2e%2E/b/.%2e/c").segments(), ["c"]);
		assert!(RouteUrl::parse("/../..").segments().is_empty());
		// A value that only contains dots is an ordinary segment.
		assert_eq!(RouteUrl::parse("/.../..a").segments(), ["...", "..a"]);
	}

	#[test]
	fn only_empty_and_dot_values_are_not_segments() {
		for value in ["", ".", ".."] {
			assert!(!is_segment(value), "{value:?}");
		}
		for value in ["...", ".a", "%2e", " "] {
			assert!(is_segment(value), "{value:?}");
		}
	}

	#[test]
	fn hrefs_escape_what_would_change_their_meaning() {
		let href = Href::from_parts(
			&["a b".to_owned(), "x/y".to_owned(), "ünï".to_owned()],
			&[("q", "1&2=3+4".to_owned())],
		);
		assert_eq!(href.as_str(), "/a%20b/x%2Fy/%C3%BCn%C3%AF?q=1%262%3D3%2B4");
		let url = RouteUrl::parse(href.as_str());
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
