//! Every route renders inside its layouts on the headless backend: the
//! screens are valid trees with valid Tailwind classes, and they show what
//! their route says.

use file_routes_example::routes::About;
use file_routes_example::routes::Compose;
use file_routes_example::routes::Help;
use file_routes_example::routes::Map;
use file_routes_example::routes::Navigation;
use file_routes_example::routes::Note;
use file_routes_example::routes::Photo;
use file_routes_example::routes::Route;
use file_routes_example::routes::Search;
use file_routes_example::routes::Section;
use file_routes_example::routes::Today;
use leptoskit::native::HeadlessBackend;
use leptoskit::native::Mutation;
use leptoskit::native::NativeSurface;
use leptoskit_router::NotFound;
use leptoskit_router::Routable;

/// Commits a route's screen at phone size and returns the text it shows.
fn render(route: impl Into<Route>) -> Vec<String> {
	let navigation = Navigation::new(route);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	surface.set_root(&navigation.render());
	assert!(surface.root().is_some(), "the screen mounts");
	surface
		.backend()
		.log()
		.iter()
		.filter_map(|mutation| {
			match mutation {
				Mutation::Create { props, .. } => props.text.clone(),
				_ => None,
			}
		})
		.collect()
}

fn shows(route: impl Into<Route>, text: &str) {
	let shown = render(route);
	assert!(shown.iter().any(|line| line == text), "{text}: {shown:?}");
}

#[test]
fn every_route_renders_its_page() {
	shows(Today, "Today");
	shows(Map { near: None }, "Map");
	shows(Route::Notes, "Note 2");
	shows(
		Note {
			note: 42,
			highlight: Some(3),
		},
		"Note 42, line 3",
	);
	shows(
		Note {
			note: 7,
			highlight: None,
		},
		"Note 7",
	);
	shows(Route::You, "You");
	shows(Route::Settings, "Settings");
	shows(
		Section {
			section: "privacy".to_owned(),
		},
		"privacy",
	);
	shows(
		Section {
			section: "privacy".to_owned(),
		},
		"Settings › privacy",
	);
	shows(Route::Settings, "Choose a section");
	shows(Photo { photo: 9 }, "Photo 9");
	shows(
		Route::PhotosNotFound(NotFound {
			path: vec!["beach".to_owned()],
		}),
		"This photo doesn't exist.",
	);
	shows(
		Search {
			q: String::new(),
			tag: Vec::new(),
			page: 0,
		},
		"Search notes",
	);
	let search = render(Search {
		q: "sync".to_owned(),
		tag: vec!["work".to_owned()],
		page: 1,
	});
	for text in ["Notes matching “sync”", "#work", "Page 2"] {
		assert!(search.iter().any(|line| line == text), "{text}: {search:?}");
	}
	shows(
		Help {
			topic: vec!["sync".to_owned(), "ios".to_owned()],
		},
		"sync › ios",
	);
	shows(Route::SignIn, "Sign in");
	shows(
		Route::NotFound(NotFound {
			path: vec!["nowhere".to_owned()],
		}),
		"This page doesn't exist.",
	);
	shows(About { lang: None }, "About Field");
	shows(
		About {
			lang: Some("fr".to_owned()),
		},
		"À propos de Field",
	);
	// The editor has no text, only a placeholder.
	render(Compose { reply_to: None });
}

#[test]
fn routes_in_tabs_get_the_tab_bar_and_others_do_not() {
	for route in [Route::from(Today), Route::Notes, Route::You] {
		let shown = render(route);
		assert!(shown.iter().any(|line| line == "Notes"), "{shown:?}");
		assert!(shown.iter().any(|line| line == "You"), "{shown:?}");
	}
	let shown = render(Route::Settings);
	assert!(!shown.iter().any(|line| line == "Today"), "{shown:?}");
}

#[test]
fn every_route_struct_converts_into_its_route() {
	let cases: [(Route, &str); 9] = [
		(Compose { reply_to: None }.into(), "/compose"),
		(
			Search {
				q: "a b".to_owned(),
				tag: Vec::new(),
				page: 0,
			}
			.into(),
			"/search?q=a%20b",
		),
		(Map { near: None }.into(), "/map"),
		(
			Note {
				note: 1,
				highlight: None,
			}
			.into(),
			"/notes/1",
		),
		(Photo { photo: 2 }.into(), "/photos/2"),
		(
			Section {
				section: "a".to_owned(),
			}
			.into(),
			"/settings/a",
		),
		(
			Help {
				topic: vec!["b".to_owned()],
			}
			.into(),
			"/help/b",
		),
		(About { lang: None }.into(), "/about"),
		(
			Route::NotFound(NotFound {
				path: vec!["c".to_owned()],
			}),
			"/c",
		),
	];
	for (route, href) in cases {
		assert_eq!(route.href().as_str(), href);
	}
}
