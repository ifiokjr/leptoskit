//! Navigating the example the way a person would: tapping its links on the
//! headless backend, with the router's in-memory history.

use file_routes_example::routes::Compose;
use file_routes_example::routes::Navigation;
use file_routes_example::routes::Note;
use file_routes_example::routes::Route;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

fn app(navigation: &Navigation) -> WidgetTester {
	let render = navigation.clone();
	WidgetTester::new(move || render.render(), 390.0, 844.0).expect("the app mounts")
}

#[test]
fn links_navigate_and_back_returns() {
	let navigation = Navigation::new(Route::Notes);
	let mut tester = app(&navigation);
	assert!(!navigation.can_go_back());
	assert!(!navigation.back(), "nothing to go back to");

	tester
		.tap(&Finder::label("Open note 2"))
		.expect("tap the link");
	tester.pump();
	assert_eq!(
		navigation.current(),
		Route::Note(Note {
			note: 2,
			highlight: None,
		})
	);
	tester
		.find(&Finder::text("Note 2"))
		.expect("the note shows");

	tester.tap(&Finder::label("Reply")).expect("reply");
	tester.pump();
	assert_eq!(
		navigation.current(),
		Route::from(Compose { reply_to: Some(2) })
	);

	assert!(navigation.back());
	assert!(navigation.back());
	tester.pump();
	assert_eq!(navigation.current(), Route::Notes);
	assert!(!navigation.can_go_back());
}

#[test]
fn the_tab_bar_switches_tabs_and_a_page_goes_back_by_itself() {
	let navigation = Navigation::new(Route::Notes);
	let mut tester = app(&navigation);
	tester.tap(&Finder::label("You")).expect("the You tab");
	tester.pump();
	assert_eq!(navigation.current(), Route::You);

	// The You page is a `#[component]` with a link of its own.
	tester
		.tap(&Finder::label("Open settings"))
		.expect("settings");
	tester.pump();
	assert_eq!(navigation.current(), Route::Settings);

	navigation.replace(Note {
		note: 5,
		highlight: None,
	});
	tester.pump();
	tester
		.tap(&Finder::text("Back"))
		.expect("the note's back button");
	tester.pump();
	assert_eq!(navigation.current(), Route::You, "replace kept You beneath");
}

#[test]
fn navigating_to_the_current_route_adds_no_history() {
	let navigation = Navigation::new(Route::Notes);
	navigation.navigate(Route::Notes);
	assert!(!navigation.can_go_back());
	assert!(format!("{navigation:?}").contains("Notes"));
}
