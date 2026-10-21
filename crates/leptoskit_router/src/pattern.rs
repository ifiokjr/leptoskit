//! URL patterns: matching a path against a route, and writing a route's path.

use std::fmt;

use crate::Href;
use crate::ParamSink;
use crate::RouteParams;

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

/// Values captured from a path, by parameter name.
pub type Captures = Vec<(&'static str, Vec<String>)>;

impl Pattern {
	/// A pattern over `segments`.
	pub const fn new(segments: &'static [Segment]) -> Self {
		Self { segments }
	}

	/// The pattern's segments.
	pub const fn segments(&self) -> &'static [Segment] {
		self.segments
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
	pub fn href<P: RouteParams>(&self, params: &P) -> Href {
		let mut sink = ParamSink::default();
		params.encode(&mut sink);
		let mut path = Vec::new();
		for segment in self.segments {
			match *segment {
				Segment::Static(text) => path.push(text.to_owned()),
				Segment::Param(name)
				| Segment::OptionalParam(name)
				| Segment::Rest(name)
				| Segment::OptionalRest(name) => path.extend(sink.take(name)),
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
/// the rest of the pattern matches what is left.
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
		if count > 0 {
			captures.push((name, path[..count].to_vec()));
		}
		if capture_into(pattern, &path[count..], captures) {
			return true;
		}
		if count > 0 {
			captures.pop();
		}
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
		let english = pattern.capture(&path(&["about"])).unwrap();
		assert_eq!(values(&english, "lang"), None);
		let french = pattern.capture(&path(&["fr", "about"])).unwrap();
		assert_eq!(values(&french, "lang"), Some(path(&["fr"])));
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
			None
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
