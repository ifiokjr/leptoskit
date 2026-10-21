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

const NOTE: Pattern = Pattern::new(&[Segment::Static("notes"), Segment::Param("note")]);
const HELP: Pattern = Pattern::new(&[Segment::Static("help"), Segment::Rest("topic")]);

fn read<P: RouteParams>(pattern: &Pattern, url: &str) -> P {
	let url = RouteUrl::parse(url).unwrap();
	let captures = pattern.capture(url.segments()).unwrap();
	decode(&captures, &url).unwrap()
}

#[test]
fn lists_fields_in_declaration_order_without_raw_prefixes() {
	assert_eq!(
		NoteRoute::FIELDS,
		["note", "highlight", "tag", "page", "type"]
	);
	assert!(Today::FIELDS.is_empty());
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
}

#[test]
fn a_missing_required_value_does_not_decode() {
	let url = RouteUrl::parse("/notes/42").unwrap();
	let captures = NOTE.capture(url.segments()).unwrap();
	assert!(decode::<NoteRoute>(&captures, &url).is_err());
}
