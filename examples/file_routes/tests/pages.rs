//! Every page and layout in the example renders on the headless backend: the
//! screens are valid trees with valid Tailwind classes, and they show what
//! their route says.

use file_routes_example::routes;
use file_routes_example::routes::About;
use file_routes_example::routes::Compose;
use file_routes_example::routes::Help;
use file_routes_example::routes::Map;
use file_routes_example::routes::Note;
use file_routes_example::routes::Photo;
use file_routes_example::routes::Route;
use file_routes_example::routes::Section;
use leptoskit::native::Element;
use leptoskit::native::HeadlessBackend;
use leptoskit::native::Mutation;
use leptoskit::native::NativeSurface;
use leptoskit_router::NotFound;
use leptoskit_router::Routable;

/// Commits a screen at phone size and returns the text it shows.
fn render(screen: &Element) -> Vec<String> {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	surface.set_root(screen);
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

#[test]
fn every_page_renders() {
	let note = Note {
		note: 42,
		highlight: Some(3),
	};
	let pages = [
		(routes::tabs::index::page(), "Today"),
		(routes::tabs::map::page(), "Map"),
		(routes::tabs::notes::index::page(), "Notes"),
		(routes::tabs::notes::note::page(&note), "Note 42, line 3"),
		(routes::tabs::you::page(), "You"),
		(routes::settings::index::page(), "Settings"),
		(
			routes::settings::section::page(&Section {
				section: "privacy".to_owned(),
			}),
			"privacy",
		),
		(routes::photos::photo::page(&Photo { photo: 9 }), "Photo 9"),
		(
			routes::help::topic::page(&Help {
				topic: vec!["sync".to_owned(), "ios".to_owned()],
			}),
			"sync › ios",
		),
		(routes::auth::sign_in::page(), "Sign in"),
		(routes::not_found::page(), "This page doesn't exist."),
		(
			routes::lang::about::page(&About { lang: None }),
			"About Field",
		),
		(
			routes::lang::about::page(&About {
				lang: Some("fr".to_owned()),
			}),
			"À propos de Field",
		),
	];
	for (page, text) in pages {
		let shown = render(&page);
		assert!(shown.iter().any(|line| line == text), "{text}: {shown:?}");
	}
	// The editor has no text, only a placeholder.
	render(&routes::compose::page());
	// A note without a highlighted line says only its number.
	let plain = routes::tabs::notes::note::page(&Note {
		note: 7,
		highlight: None,
	});
	assert!(render(&plain).iter().any(|line| line == "Note 7"));
}

#[test]
fn layouts_wrap_the_screen() {
	let screen = routes::tabs::you::page();
	let shown = render(&routes::layout::layout(routes::tabs::layout::layout(
		screen,
	)));
	assert!(shown.iter().any(|line| line == "You"), "{shown:?}");
	assert!(
		shown.iter().any(|line| line == "Today · Map · Notes · You"),
		"{shown:?}"
	);
}

#[test]
fn every_route_struct_converts_into_its_route() {
	let cases: [(Route, &str); 8] = [
		(Compose { reply_to: None }.into(), "/compose"),
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
