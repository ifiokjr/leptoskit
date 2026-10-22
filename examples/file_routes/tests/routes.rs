//! The generated `Route` enum parses every URL the example's files define,
//! and writes hrefs that parse back to the same route.

use file_routes_example::routes::About;
use file_routes_example::routes::Compose;
use file_routes_example::routes::Help;
use file_routes_example::routes::Map;
use file_routes_example::routes::Note;
use file_routes_example::routes::Photo;
use file_routes_example::routes::Route;
use file_routes_example::routes::Section;
use leptoskit_router::NotFound;
use leptoskit_router::Routable;

fn parse(url: &str) -> Route {
	Route::parse(url).unwrap_or_else(|error| panic!("{url}: {error}"))
}

#[test]
fn every_file_is_a_route() {
	let cases = [
		("/", Route::Today),
		("/map", Route::Map(Map { near: None })),
		(
			"/map?near=lagos",
			Route::Map(Map {
				near: Some("lagos".to_owned()),
			}),
		),
		("/notes", Route::Notes),
		(
			"/notes/42?highlight=3",
			Route::Note(Note {
				note: 42,
				highlight: Some(3),
			}),
		),
		("/you", Route::You),
		("/compose", Route::Compose(Compose { reply_to: None })),
		("/photos/9", Route::Photo(Photo { photo: 9 })),
		("/settings", Route::Settings),
		(
			"/settings/privacy",
			Route::Section(Section {
				section: "privacy".to_owned(),
			}),
		),
		(
			"/help/sync/ios",
			Route::Help(Help {
				topic: vec!["sync".to_owned(), "ios".to_owned()],
			}),
		),
		("/sign-in", Route::SignIn),
		("/about", Route::About(About { lang: None })),
		(
			"/fr/about",
			Route::About(About {
				lang: Some("fr".to_owned()),
			}),
		),
	];
	for (url, expected) in cases {
		let route = parse(url);
		assert_eq!(route, expected, "{url}");
		assert_eq!(
			route.href().as_str(),
			url,
			"{url} writes back the same href"
		);
	}
}

#[test]
fn a_static_segment_wins_over_an_optional_parameter() {
	// `[[lang]]/about.rs` could read `/settings/about` as lang = "settings";
	// the more specific `settings/[section].rs` wins.
	assert_eq!(
		parse("/settings/about"),
		Route::Section(Section {
			section: "about".to_owned(),
		})
	);
}

#[test]
fn unmatched_urls_reach_not_found_with_their_path() {
	assert_eq!(
		parse("/nowhere/at/all"),
		Route::NotFound(NotFound {
			path: vec!["nowhere".to_owned(), "at".to_owned(), "all".to_owned()],
		})
	);
	// `[note]` is a number, so `/notes/draft` matches no note.
	assert_eq!(
		parse("/notes/draft"),
		Route::NotFound(NotFound {
			path: vec!["notes".to_owned(), "draft".to_owned()],
		})
	);
	// `[...topic]` needs at least one segment.
	assert!(matches!(parse("/help"), Route::NotFound(_)));
}

#[test]
fn links_are_built_from_types() {
	let reply = Route::from(Compose { reply_to: Some(42) });
	assert_eq!(reply.href().as_str(), "/compose?reply_to=42");
	assert_eq!(parse(reply.href().as_str()), reply);
	assert_eq!(
		Route::from(file_routes_example::routes::Today),
		Route::Today
	);
}
