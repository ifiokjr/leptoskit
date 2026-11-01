//! URL patterns: matching a path against a route, and writing a route's path.

use std::fmt;

use crate::Href;
use crate::ParamSink;
use crate::RouteParams;
use crate::url::is_segment;

/// One segment of a route's URL pattern, from one name in its file path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
	/// A fixed segment, such as `notes` from `notes/`.
	Static(&'static str),
	/// One value, from `[note]`.
	Param(&'static str),
	/// One value or none, from `[[lang]]`.
	OptionalParam(&'static str),
	/// One or more values, from `[...topic]`.
	Rest(&'static str),
	/// Any number of values, from `[[...topic]]`.
	OptionalRest(&'static str),
}

/// A route's URL pattern: its file path without groups, `index` and the `.rs`
/// extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
	segments: &'static [Segment],
}

/// Values captured from a path, by parameter name. Every parameter in the
/// pattern has an entry, with no values when its optional segment is absent.
pub type Captures = Vec<(&'static str, Vec<String>)>;

impl Segment {
	/// The parameter's name, or `None` for a static segment.
	const fn param(&self) -> Option<&'static str> {
		match *self {
			Self::Static(_) => None,
			Self::Param(name)
			| Self::OptionalParam(name)
			| Self::Rest(name)
			| Self::OptionalRest(name) => Some(name),
		}
	}
}

impl Pattern {
	/// A pattern over `segments`.
	pub const fn new(segments: &'static [Segment]) -> Self {
		Self { segments }
	}

	/// Matches decoded path segments, returning the captured values. An
	/// optional segment or catch-all takes as many segments as it can while
	/// the rest of the pattern still matches.
	pub fn capture(&self, path: &[String]) -> Option<Captures> {
		let mut captures = Vec::new();
		capture_into(self.segments, path, &mut captures).then_some(captures)
	}

	/// Writes the href for a route struct: path parameters fill their
	/// segments, and every other field becomes the query string.
	///
	/// A URL path cannot hold an empty segment, `.` or `..`: browsers drop
	/// or resolve them, and an empty first segment would make the href
	/// `//host`, a link to another site. So an optional segment holding one
	/// of those values is left out, as if it were absent, and a required
	/// segment or catch-all value holding one ends the path there: the href
	/// goes to the deepest URL it can write rather than moving later values
	/// into its place.
	pub fn href<P: RouteParams>(&self, params: &P) -> Href {
		let mut sink = ParamSink::default();
		params.encode(&mut sink);
		// Every path parameter leaves the sink before the path is written, so
		// none of them lands in the query string when the path ends early.
		let values: Vec<Vec<String>> = self
			.segments
			.iter()
			.map(|segment| {
				segment
					.param()
					.map(|name| sink.take(name))
					.unwrap_or_default()
			})
			.collect();
		let mut path = Vec::new();

		for (segment, mut values) in self.segments.iter().zip(values) {
			let complete = match *segment {
				Segment::Static(text) => {
					path.push(text.to_owned());
					true
				}
				Segment::OptionalParam(_) => {
					path.extend(values.into_iter().take(1).filter(|value| is_segment(value)));
					true
				}
				Segment::Param(_) => {
					values.truncate(1);
					!values.is_empty() && extend_path(&mut path, values)
				}
				Segment::Rest(_) => !values.is_empty() && extend_path(&mut path, values),
				Segment::OptionalRest(_) => extend_path(&mut path, values),
			};

			if !complete {
				break;
			}
		}

		Href::from_parts(&path, &sink.into_query())
	}

	/// The href of a pattern with no parameters.
	pub fn static_href(&self) -> Href {
		let path: Vec<String> = self
			.segments
			.iter()
			.filter_map(|segment| {
				match segment {
					Segment::Static(text) => Some((*text).to_owned()),
					_ => None,
				}
			})
			.collect();
		Href::from_parts(&path, &[])
	}
}

/// Appends `values` to `path` up to the first one a path cannot hold, and
/// says whether it appended them all.
fn extend_path(path: &mut Vec<String>, values: Vec<String>) -> bool {
	for value in values {
		if !is_segment(&value) {
			return false;
		}

		path.push(value);
	}

	true
}

/// One shape of URL a generated route matches: its pattern with each optional
/// segment either present or absent. `leptoskit routes` expands every route
/// into its shapes and orders them all from most to least specific, so `/about`
/// matches `[[lang]]/about.rs` with no language before it can match
/// `[section].rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
	/// The present segments, with optional ones written as required ones.
	pattern: Pattern,
	/// The route's optional parameters this shape leaves out.
	absent: &'static [&'static str],
}

impl Shape {
	/// A shape over `segments`, leaving out the parameters `absent`.
	pub const fn new(segments: &'static [Segment], absent: &'static [&'static str]) -> Self {
		Self {
			pattern: Pattern::new(segments),
			absent,
		}
	}

	/// Matches decoded path segments. Each absent parameter is captured with
	/// no values, so it never reads from the query string.
	pub fn capture(&self, path: &[String]) -> Option<Captures> {
		let mut captures = self.pattern.capture(path)?;
		captures.extend(self.absent.iter().map(|name| (*name, Vec::new())));
		Some(captures)
	}
}

fn capture_into(pattern: &[Segment], path: &[String], captures: &mut Captures) -> bool {
	let Some((segment, pattern)) = pattern.split_first() else {
		return path.is_empty();
	};

	match *segment {
		Segment::Static(text) => {
			path.first().is_some_and(|first| first == text)
				&& capture_into(pattern, &path[1..], captures)
		}
		Segment::Param(name) => take(name, 1..=1, pattern, path, captures),
		Segment::OptionalParam(name) => take(name, 0..=1, pattern, path, captures),
		Segment::Rest(name) => take(name, 1..=path.len(), pattern, path, captures),
		Segment::OptionalRest(name) => take(name, 0..=path.len(), pattern, path, captures),
	}
}

/// Tries to give `name` each count of segments in `counts`, most first, until
/// the rest of the pattern matches what is left. A count of zero is still
/// captured, with no values.
fn take(
	name: &'static str,
	counts: std::ops::RangeInclusive<usize>,
	pattern: &[Segment],
	path: &[String],
	captures: &mut Captures,
) -> bool {
	for count in counts.rev() {
		if count > path.len() {
			continue;
		}

		captures.push((name, path[..count].to_vec()));

		if capture_into(pattern, &path[count..], captures) {
			return true;
		}

		captures.pop();
	}

	false
}

impl fmt::Display for Pattern {
	/// Writes the pattern the way people write routes: `/notes/:note`,
	/// `/:lang?/about`, `/help/*topic`, `/docs/*path?`.
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.segments.is_empty() {
			return formatter.write_str("/");
		}

		for segment in self.segments {
			match segment {
				Segment::Static(text) => write!(formatter, "/{text}")?,
				Segment::Param(name) => write!(formatter, "/:{name}")?,
				Segment::OptionalParam(name) => write!(formatter, "/:{name}?")?,
				Segment::Rest(name) => write!(formatter, "/*{name}")?,
				Segment::OptionalRest(name) => write!(formatter, "/*{name}?")?,
			}
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn path(segments: &[&str]) -> Vec<String> {
		segments
			.iter()
			.map(|segment| (*segment).to_owned())
			.collect()
	}

	fn values(captures: &Captures, name: &str) -> Option<Vec<String>> {
		captures
			.iter()
			.find(|(key, _)| *key == name)
			.map(|(_, values)| values.clone())
	}

	#[test]
	fn matches_static_and_dynamic_segments() {
		let pattern = Pattern::new(&[Segment::Static("notes"), Segment::Param("note")]);
		let captures = pattern.capture(&path(&["notes", "42"])).unwrap();
		assert_eq!(values(&captures, "note"), Some(path(&["42"])));
		assert!(pattern.capture(&path(&["notes"])).is_none());
		assert!(pattern.capture(&path(&["notes", "42", "x"])).is_none());
		assert!(pattern.capture(&path(&["photos", "42"])).is_none());
	}

	#[test]
	fn an_optional_segment_steps_aside_when_the_rest_needs_it() {
		let pattern = Pattern::new(&[Segment::OptionalParam("lang"), Segment::Static("about")]);
		// Absent, the segment is still captured, so the query cannot fill it.
		let english = pattern.capture(&path(&["about"])).unwrap();
		assert_eq!(values(&english, "lang"), Some(Vec::new()));
		let french = pattern.capture(&path(&["fr", "about"])).unwrap();
		assert_eq!(values(&french, "lang"), Some(path(&["fr"])));
	}

	#[test]
	fn a_shape_matches_only_its_own_segments_and_captures_what_it_leaves_out() {
		let absent = Shape::new(&[Segment::Static("about")], &["lang"]);
		let captures = absent.capture(&path(&["about"])).unwrap();
		assert_eq!(values(&captures, "lang"), Some(Vec::new()));
		assert!(absent.capture(&path(&["fr", "about"])).is_none());

		let present = Shape::new(&[Segment::Param("lang"), Segment::Static("about")], &[]);
		assert!(present.capture(&path(&["about"])).is_none());
		let captures = present.capture(&path(&["fr", "about"])).unwrap();
		assert_eq!(values(&captures, "lang"), Some(path(&["fr"])));
	}

	#[test]
	fn catch_alls_take_one_or_more_or_any() {
		let rest = Pattern::new(&[Segment::Static("help"), Segment::Rest("topic")]);
		assert!(rest.capture(&path(&["help"])).is_none());
		let captures = rest.capture(&path(&["help", "sync", "ios"])).unwrap();
		assert_eq!(values(&captures, "topic"), Some(path(&["sync", "ios"])));

		let any = Pattern::new(&[Segment::Static("docs"), Segment::OptionalRest("path")]);
		assert_eq!(
			values(&any.capture(&path(&["docs"])).unwrap(), "path"),
			Some(Vec::new())
		);

		let middle = Pattern::new(&[Segment::Rest("dirs"), Segment::Static("edit")]);
		let captures = middle.capture(&path(&["a", "b", "edit"])).unwrap();
		assert_eq!(values(&captures, "dirs"), Some(path(&["a", "b"])));
	}

	#[test]
	fn displays_like_a_route_table() {
		let pattern = Pattern::new(&[
			Segment::OptionalParam("lang"),
			Segment::Static("docs"),
			Segment::Param("id"),
			Segment::Rest("rest"),
			Segment::OptionalRest("more"),
		]);
		assert_eq!(pattern.to_string(), "/:lang?/docs/:id/*rest/*more?");
		assert_eq!(Pattern::new(&[]).to_string(), "/");
	}

	#[test]
	fn a_static_href_skips_parameters() {
		let pattern = Pattern::new(&[
			Segment::Static("settings"),
			Segment::OptionalParam("section"),
		]);
		assert_eq!(pattern.static_href().as_str(), "/settings");
		assert_eq!(Pattern::new(&[]).static_href().as_str(), "/");
	}
}
