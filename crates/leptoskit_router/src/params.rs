//! Route parameters: reading a route struct from a matched URL, and writing it
//! back.

use std::fmt::Display;
use std::str::FromStr;

/// A route struct's fields, read from and written to a URL.
///
/// `#[derive(Route)]` implements it. The generated `routes.rs` decides which
/// fields are path segments: a field named after a `[segment]` in the route's
/// file path comes from the path, and every other field comes from the query
/// string.
pub trait RouteParams: Sized {
	/// Every field name, in declaration order.
	const FIELDS: &'static [&'static str];

	/// Reads the struct from a matched URL.
	///
	/// # Errors
	///
	/// Returns [`ParamError`] when a required value is missing or a value does
	/// not parse.
	fn decode(params: &Params<'_>) -> Result<Self, ParamError>;

	/// Writes every field, in declaration order.
	fn encode(&self, sink: &mut ParamSink);
}

/// The values of a matched URL, by name. A path segment captured under a name
/// shadows any query value with the same name.
#[derive(Debug)]
pub struct Params<'a> {
	captures: &'a [(&'static str, Vec<String>)],
	query: &'a [(String, String)],
}

impl<'a> Params<'a> {
	pub(crate) fn new(
		captures: &'a [(&'static str, Vec<String>)],
		query: &'a [(String, String)],
	) -> Self {
		Self { captures, query }
	}

	fn values(&self, name: &str) -> Vec<&'a str> {
		if let Some((_, values)) = self.captures.iter().find(|(key, _)| *key == name) {
			return values.iter().map(String::as_str).collect();
		}
		self.query
			.iter()
			.filter(|(key, _)| key == name)
			.map(|(_, value)| value.as_str())
			.collect()
	}

	/// Reads a required value. When a query key repeats, the first value
	/// wins.
	///
	/// # Errors
	///
	/// Returns [`ParamError::Missing`] when there is no value, and
	/// [`ParamError::Invalid`] when it does not parse.
	pub fn one<T>(&self, name: &'static str) -> Result<T, ParamError>
	where
		T: FromStr,
		T::Err: Display,
	{
		self.optional(name)?.ok_or(ParamError::Missing { name })
	}

	/// Reads a value that may be absent.
	///
	/// # Errors
	///
	/// Returns [`ParamError::Invalid`] when a value is present but does not
	/// parse.
	pub fn optional<T>(&self, name: &'static str) -> Result<Option<T>, ParamError>
	where
		T: FromStr,
		T::Err: Display,
	{
		self.values(name)
			.first()
			.map(|value| parse(name, value))
			.transpose()
	}

	/// Reads a value, or `T::default()` when it is absent.
	///
	/// # Errors
	///
	/// Returns [`ParamError::Invalid`] when a value is present but does not
	/// parse.
	pub fn one_or_default<T>(&self, name: &'static str) -> Result<T, ParamError>
	where
		T: FromStr + Default,
		T::Err: Display,
	{
		Ok(self.optional(name)?.unwrap_or_default())
	}

	/// Reads every value: the segments of a catch-all, or each repeat of a
	/// query key.
	///
	/// # Errors
	///
	/// Returns [`ParamError::Invalid`] when any value does not parse.
	pub fn many<T>(&self, name: &'static str) -> Result<Vec<T>, ParamError>
	where
		T: FromStr,
		T::Err: Display,
	{
		self.values(name)
			.into_iter()
			.map(|value| parse(name, value))
			.collect()
	}
}

fn parse<T>(name: &'static str, value: &str) -> Result<T, ParamError>
where
	T: FromStr,
	T::Err: Display,
{
	value.parse().map_err(|error: T::Err| {
		ParamError::Invalid {
			name,
			value: value.to_owned(),
			reason: error.to_string(),
		}
	})
}

/// Why a URL's values could not become a route struct.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParamError {
	/// A required value is absent.
	#[error("missing parameter `{name}`")]
	Missing {
		/// The field's name.
		name: &'static str,
	},
	/// A value is present but does not parse as the field's type.
	#[error("parameter `{name}` has an invalid value `{value}`: {reason}")]
	Invalid {
		/// The field's name.
		name: &'static str,
		/// The value from the URL.
		value: String,
		/// The parse error.
		reason: String,
	},
}

/// Collects a route struct's values by name, for writing its href.
#[derive(Debug, Default)]
pub struct ParamSink {
	values: Vec<(&'static str, Vec<String>)>,
}

impl ParamSink {
	/// Writes a required value.
	pub fn one<T: Display>(&mut self, name: &'static str, value: &T) {
		self.values.push((name, vec![value.to_string()]));
	}

	/// Writes a value when it is present.
	pub fn optional<T: Display>(&mut self, name: &'static str, value: Option<&T>) {
		if let Some(value) = value {
			self.one(name, value);
		}
	}

	/// Writes a value unless it equals `T::default()`, which a URL leaves
	/// out.
	pub fn unless_default<T: Display + Default + PartialEq>(
		&mut self,
		name: &'static str,
		value: &T,
	) {
		if *value != T::default() {
			self.one(name, value);
		}
	}

	/// Writes every value: the segments of a catch-all, or a repeated query
	/// key.
	pub fn many<T: Display>(&mut self, name: &'static str, values: &[T]) {
		if !values.is_empty() {
			self.values
				.push((name, values.iter().map(ToString::to_string).collect()));
		}
	}

	/// Removes and returns the values written under `name`.
	pub(crate) fn take(&mut self, name: &str) -> Vec<String> {
		self.values
			.iter()
			.position(|(key, _)| *key == name)
			.map(|index| self.values.remove(index).1)
			.unwrap_or_default()
	}

	/// The values left after the path took its segments: the query string.
	pub(crate) fn into_query(self) -> Vec<(&'static str, String)> {
		self.values
			.into_iter()
			.flat_map(|(name, values)| values.into_iter().map(move |value| (name, value)))
			.collect()
	}
}

/// The unmatched part of a URL, given to a `+not-found.rs` route: the
/// segments after the directory that holds the `+not-found.rs` file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotFound {
	/// The unmatched path segments.
	pub path: Vec<String>,
}

impl RouteParams for NotFound {
	const FIELDS: &'static [&'static str] = &["path"];

	fn decode(params: &Params<'_>) -> Result<Self, ParamError> {
		Ok(Self {
			path: params.many("path")?,
		})
	}

	fn encode(&self, sink: &mut ParamSink) {
		sink.many("path", &self.path);
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
	fn a_path_capture_shadows_the_query() {
		let captures = [("id", vec!["7".to_owned()])];
		let query = query(&[("id", "9"), ("page", "2")]);
		let params = Params::new(&captures, &query);
		assert_eq!(params.one::<u32>("id"), Ok(7));
		assert_eq!(params.one::<u32>("page"), Ok(2));
	}

	#[test]
	fn reports_missing_and_invalid_values() {
		let query = query(&[("page", "two")]);
		let params = Params::new(&[], &query);
		assert_eq!(
			params.one::<u32>("id"),
			Err(ParamError::Missing { name: "id" })
		);
		assert!(matches!(
			params.one::<u32>("page"),
			Err(ParamError::Invalid { name: "page", .. })
		));
		assert_eq!(params.optional::<u32>("id"), Ok(None));
		assert_eq!(params.one_or_default::<u32>("id"), Ok(0));
	}

	#[test]
	fn reads_every_repeat() {
		let query = query(&[("tag", "a"), ("tag", "b")]);
		let params = Params::new(&[], &query);
		assert_eq!(
			params.many::<String>("tag"),
			Ok(vec!["a".to_owned(), "b".to_owned()])
		);
		assert_eq!(params.one::<String>("tag"), Ok("a".to_owned()));
		assert_eq!(params.many::<String>("none"), Ok(Vec::new()));
	}

	#[test]
	fn not_found_writes_its_unmatched_path() {
		let mut sink = ParamSink::default();
		NotFound {
			path: vec!["a".to_owned(), "b".to_owned()],
		}
		.encode(&mut sink);
		assert_eq!(sink.take("path"), ["a", "b"]);
	}

	#[test]
	fn the_sink_leaves_out_absent_and_default_values() {
		let mut sink = ParamSink::default();
		sink.one("id", &7);
		sink.optional::<u32>("page", None);
		sink.unless_default("sort", &0_u8);
		sink.unless_default("limit", &50_u8);
		sink.many::<String>("tag", &[]);
		assert_eq!(sink.take("id"), ["7"]);
		assert_eq!(sink.into_query(), [("limit", "50".to_owned())]);
	}
}
