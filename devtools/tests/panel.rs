//! The browser `DevTools` panel's shell on the headless backend: the bar the
//! hub's frame sits under, what it says while there is no hub, and the port
//! field.

use leptoskit_devtools_ui::panel;
use leptoskit_devtools_ui::panel::Hub;
use leptoskit_devtools_ui::panel::Shell;
use leptoskit_extension::PageName;
use leptoskit_extension::Surface;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

fn reset(shell: Shell) {
	panel::update(|current| *current = shell);
}

fn tester() -> WidgetTester {
	WidgetTester::new(panel::page, 800.0, 400.0).unwrap()
}

fn missing() -> Shell {
	Shell {
		hub: Hub::Missing,
		..Shell::default()
	}
}

#[test]
fn the_hub_s_frame_starts_just_below_the_bar() {
	reset(Shell::default());
	let bar = tester().find(&Finder::test_id("bar")).unwrap();
	assert_eq!((bar.frame.y, bar.frame.width), (0.0, 800.0));
	assert_eq!(format!("{}px", bar.frame.height), panel::FRAME_TOP);
}

#[test]
fn the_panel_looks_for_the_hub_on_its_default_port() {
	reset(Shell::default());
	let tester = tester();
	tester
		.find(&Finder::text("Looking for the hub on 127.0.0.1:7340…"))
		.unwrap();
	assert!(
		tester
			.find(&Finder::text("No DevTools hub is running"))
			.is_err()
	);
}

#[test]
fn without_a_hub_the_panel_says_how_to_start_one() {
	reset(missing());
	let tester = tester();
	tester
		.find(&Finder::text("No hub on 127.0.0.1:7340"))
		.unwrap();
	tester
		.find(&Finder::text("No DevTools hub is running"))
		.unwrap();
	tester
		.find(&Finder::text(
			"Nothing answers on 127.0.0.1:7340. Run leptoskit dev in your app's folder: it \
			 starts the hub and prints its address, such as [devtools] hub at \
			 http://127.0.0.1:7340. If it prints another port, enter it above.",
		))
		.unwrap();
	tester.find(&Finder::text("Look again")).unwrap();
	assert!(
		tester
			.find(&Finder::text(
				"This page isn't in the browser's developer tools, so it has no tab to inspect. \
				 Open the developer tools on your app's tab and choose the leptoskit panel."
			))
			.is_err(),
		"inside the developer tools"
	);
}

#[test]
fn a_page_outside_the_developer_tools_says_where_to_open_it() {
	reset(Shell {
		inspecting: false,
		..missing()
	});
	tester()
		.find(&Finder::text(
			"This page isn't in the browser's developer tools, so it has no tab to inspect. Open \
			 the developer tools on your app's tab and choose the leptoskit panel.",
		))
		.unwrap();
}

#[test]
fn a_found_hub_leaves_the_panel_to_its_frame() {
	reset(Shell {
		hub: Hub::Found("0.1.0".into()),
		..Shell::default()
	});
	let tester = tester();
	tester
		.find(&Finder::text("leptoskit 0.1.0 · 127.0.0.1:7340"))
		.unwrap();
	assert!(
		tester
			.find(&Finder::text("No DevTools hub is running"))
			.is_err()
	);
}

#[test]
fn a_typed_port_is_looked_for_and_a_bad_one_is_refused() {
	reset(missing());
	let mut tester = tester();
	tester
		.enter_text(&Finder::label("Hub port"), " 7341 ")
		.unwrap();
	tester.tap(&Finder::text("Connect")).unwrap();
	assert_eq!(
		panel::peek(|shell| (shell.port, shell.draft.clone(), shell.hub.clone())),
		(7341, "7341".to_owned(), Hub::Looking)
	);
	tester
		.find(&Finder::text("Looking for the hub on 127.0.0.1:7341…"))
		.unwrap();

	for text in ["70000", "0", "port", ""] {
		tester.enter_text(&Finder::label("Hub port"), text).unwrap();
		assert!(
			tester.tap(&Finder::text("Connect")).is_err(),
			"{text:?} isn't a port"
		);
		panel::connect();
		assert_eq!(panel::peek(|shell| shell.port), 7341, "{text:?}");
	}
}

#[test]
fn looking_again_starts_over() {
	reset(missing());
	let mut tester = tester();
	tester.tap(&Finder::text("Look again")).unwrap();
	assert_eq!(panel::peek(|shell| shell.hub.clone()), Hub::Looking);
}

#[test]
fn the_hub_is_known_by_the_version_it_answers_with() {
	assert_eq!(
		Hub::from_answer(r#"{"version":"0.1.0","protocol":1,"interface":"local"}"#),
		Hub::Found("0.1.0".into())
	);
	for answer in ["", "<html></html>", "{}", r#"{"version":1}"#] {
		assert_eq!(Hub::from_answer(answer), Hub::Missing, "{answer:?}");
	}
}

#[test]
fn the_page_that_adds_the_panel_shows_nothing() {
	let render = panel::main(Surface::Page {
		name: PageName::Devtools,
		route: panel::REGISTER_ROUTE.into(),
	});
	let tester = WidgetTester::new(render, 800.0, 400.0).unwrap();
	assert!(tester.find(&Finder::test_id("bar")).is_err());
	assert!(tester.find_all(&Finder::text(panel::TITLE)).is_empty());

	reset(Shell::default());
	let render = panel::main(Surface::Page {
		name: PageName::DevtoolsPanel,
		route: panel::PANEL_ROUTE.into(),
	});
	let tester = WidgetTester::new(render, 800.0, 400.0).unwrap();
	tester.find(&Finder::test_id("bar")).unwrap();
}

#[test]
fn the_configuration_opens_the_panel_s_routes() {
	let config = include_str!("../leptoskit.toml");
	assert!(config.contains(&format!("route = \"{}\"", panel::REGISTER_ROUTE)));
	assert!(config.contains(&format!("panel = \"{}\"", panel::PANEL_ROUTE)));
}
