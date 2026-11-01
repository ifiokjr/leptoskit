//! `#[derive(Route)]` reads and writes each field kind.

use leptoskit_router::__private::decode;
use leptoskit_router::Pattern;
use leptoskit_router::Route;
use leptoskit_router::RouteParams;
use leptoskit_router::RouteUrl;
use leptoskit_router::Segment;

#[derive(Route, Clone, Debug, PartialEq)]
struct NoteRoute {
	note: u64,
	highlight: Option<u32>,
	tag: Vec<String>,
	#[route(default)]
	page: u32,
	r#type: String,
}

#[derive(Route, Clone, Debug, PartialEq)]
struct Today;

#[derive(Route, Clone, Debug, PartialEq)]
struct Help {
	topic: Vec<String>,
}

#[derive(Route, Clone, Debug, PartialEq)]
struct About {
	lang: Option<String>,
}

#[derive(Route, Clone, Debug, PartialEq)]
struct Docs {
	path: Vec<String>,
}

#[derive(Route, Clone, Debug, PartialEq)]
struct Pair {
	a: String,
	b: String,
	#[route(default)]
	page: u32,
}

const NOTE: Pattern = Pattern::new(&[Segment::Static("notes"), Segment::Param("note")]);
const HELP: Pattern = Pattern::new(&[Segment::Static("help"), Segment::Rest("topic")]);
const ABOUT: Pattern = Pattern::new(&[Segment::OptionalParam("lang"), Segment::Static("about")]);
const DOCS: Pattern = Pattern::new(&[Segment::Static("docs"), Segment::OptionalRest("path")]);
const PAIR: Pattern = Pattern::new(&[Segment::Param("a"), Segment::Param("b")]);

fn read<P: RouteParams>(pattern: &Pattern, url: &str) -> P {
	let url = RouteUrl::parse(url);
	let captures = pattern.capture(url.segments()).unwrap();
	decode(&captures, &url).unwrap()
}

#[test]
fn lists_fields_in_declaration_order_without_raw_prefixes() {
	assert_eq!(
		NoteRoute::FIELDS,
		["note", "highlight", "tag", "page", "type"]
	);
	assert_eq!(Today::FIELDS, [] as [&str; 0]);
}

#[test]
fn reads_path_and_query_values_by_kind() {
	let note: NoteRoute = read(&NOTE, "/notes/42?highlight=3&tag=a&tag=b&type=draft");
	assert_eq!(
		note,
		NoteRoute {
			note: 42,
			highlight: Some(3),
			tag: vec!["a".to_owned(), "b".to_owned()],
			page: 0,
			r#type: "draft".to_owned(),
		}
	);
}

#[test]
fn a_query_value_that_does_not_parse_is_absent() {
	let note: NoteRoute = read(&NOTE, "/notes/42?highlight=x&page=two&type=draft");
	assert_eq!(note.highlight, None);
	assert_eq!(note.page, 0);
}

#[test]
fn an_absent_optional_segment_never_reads_the_query() {
	assert_eq!(
		read::<About>(&ABOUT, "/about?lang=fr"),
		About { lang: None }
	);
	assert_eq!(
		read::<Docs>(&DOCS, "/docs?path=a"),
		Docs { path: Vec::new() }
	);
}

#[test]
fn writes_an_href_that_reads_back() {
	let note = NoteRoute {
		note: 42,
		highlight: None,
		tag: vec!["a b".to_owned()],
		page: 2,
		r#type: "draft".to_owned(),
	};
	let href = NOTE.href(&note);
	assert_eq!(href.as_str(), "/notes/42?tag=a%20b&page=2&type=draft");
	assert_eq!(read::<NoteRoute>(&NOTE, href.as_str()), note);
}

#[test]
fn a_catch_all_round_trips_its_segments() {
	let help = Help {
		topic: vec!["sync".to_owned(), "ios".to_owned()],
	};
	assert_eq!(HELP.href(&help).as_str(), "/help/sync/ios");
	assert_eq!(read::<Help>(&HELP, "/help/sync/ios"), help);

	let empty = Docs { path: Vec::new() };
	assert_eq!(DOCS.href(&empty).as_str(), "/docs");
	assert_eq!(read::<Docs>(&DOCS, "/docs"), empty);
}

#[test]
fn an_optional_segment_a_path_cannot_hold_is_absent() {
	for lang in ["", ".", ".."] {
		let about = About {
			lang: Some(lang.to_owned()),
		};
		assert_eq!(ABOUT.href(&about).as_str(), "/about", "{lang:?}");
	}

	let href = ABOUT.href(&About {
		lang: Some("fr".to_owned()),
	});
	assert_eq!(href.as_str(), "/fr/about");
}

#[test]
fn a_required_value_a_path_cannot_hold_ends_the_path() {
	// Never `//evil.com`, a link to another site, and `b` stays out of the
	// query string.
	let pair = Pair {
		a: String::new(),
		b: "evil.com".to_owned(),
		page: 2,
	};
	assert_eq!(PAIR.href(&pair).as_str(), "/?page=2");
	let pair = Pair {
		a: "x".to_owned(),
		b: "..".to_owned(),
		page: 0,
	};
	assert_eq!(PAIR.href(&pair).as_str(), "/x");

	let help = Help {
		topic: vec!["a".to_owned(), String::new(), "b".to_owned()],
	};
	assert_eq!(HELP.href(&help).as_str(), "/help/a");
	let help = Help {
		topic: vec![".".to_owned()],
	};
	assert_eq!(HELP.href(&help).as_str(), "/help");
	let docs = Docs {
		path: vec!["guide".to_owned(), "..".to_owned()],
	};
	assert_eq!(DOCS.href(&docs).as_str(), "/docs/guide");
}

#[test]
fn a_missing_required_value_does_not_decode() {
	let url = RouteUrl::parse("/notes/42");
	let captures = NOTE.capture(url.segments()).unwrap();
	assert!(decode::<NoteRoute>(&captures, &url).is_err());
}
