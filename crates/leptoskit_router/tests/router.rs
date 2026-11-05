//! A native app's router: navigation, the in-memory history `back` returns
//! through, and links that navigate when pressed.

use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Text;
use leptoskit_router::__private::decode;
use leptoskit_router::Href;
use leptoskit_router::NoMatch;
use leptoskit_router::Pattern;
use leptoskit_router::Render;
use leptoskit_router::Routable;
use leptoskit_router::Route;
use leptoskit_router::RouteUrl;
use leptoskit_router::Router;
use leptoskit_router::Segment;

#[derive(Route, Clone, Debug, PartialEq)]
struct NoteRoute {
	note: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum Screen {
	Home,
	Settings,
	Note(NoteRoute),
}

const HOME: Pattern = Pattern::new(&[]);
const SETTINGS: Pattern = Pattern::new(&[Segment::Static("settings")]);
const NOTE: Pattern = Pattern::new(&[Segment::Static("notes"), Segment::Param("note")]);

impl Routable for Screen {
	fn from_url(url: &RouteUrl) -> Result<Self, NoMatch> {
		if let Some(captures) = NOTE.capture(url.segments()) {
			return decode(&captures, url).map(Self::Note).map_err(|_| NoMatch);
		}

		if SETTINGS.capture(url.segments()).is_some() {
			return Ok(Self::Settings);
		}

		HOME.capture(url.segments())
			.map(|_| Self::Home)
			.ok_or(NoMatch)
	}

	fn href(&self) -> Href {
		match self {
			Self::Home => HOME.static_href(),
			Self::Settings => SETTINGS.static_href(),
			Self::Note(note) => NOTE.href(note),
		}
	}
}

impl Render for Screen {
	fn render(&self, _navigation: &Router<Self>) -> Element {
		Text(match self {
			Self::Home => "Home".to_owned(),
			Self::Settings => "Settings".to_owned(),
			Self::Note(note) => format!("Note {}", note.note),
		})
	}
}

fn note(note: u64) -> Screen {
	Screen::Note(NoteRoute { note })
}

#[test]
fn back_returns_through_each_route_navigated_from() {
	let router = Router::<Screen>::new(Screen::Home);
	assert!(!router.can_go_back());
	assert!(!router.back(), "there is nothing to go back to");
	assert_eq!(router.current(), Screen::Home);

	router.navigate(Screen::Settings);
	router.navigate(note(4));
	assert_eq!(router.current(), note(4));
	assert!(router.can_go_back());

	assert!(router.back());
	assert_eq!(router.current(), Screen::Settings);
	assert!(router.back());
	assert_eq!(router.current(), Screen::Home);
	assert!(!router.can_go_back());
}

#[test]
fn navigating_to_the_current_route_adds_no_history() {
	let router = Router::<Screen>::new(Screen::Settings);
	router.navigate(Screen::Settings);
	assert!(!router.can_go_back());
}

#[test]
fn replace_takes_the_current_routes_place() {
	let router = Router::<Screen>::new(Screen::Home);
	router.navigate(Screen::Settings);
	router.replace(note(9));
	assert_eq!(router.current(), note(9));

	// Settings was replaced, so back skips it.
	assert!(router.back());
	assert_eq!(router.current(), Screen::Home);
}

#[test]
fn every_clone_navigates_the_same_history() {
	let router = Router::<Screen>::new(Screen::Home);
	let clone = router.clone();
	clone.navigate(note(1));
	assert_eq!(router.current(), note(1));
	assert!(router.can_go_back());
	assert_eq!(
		format!("{router:?}"),
		"Router { current: Note(NoteRoute { note: 1 }), .. }"
	);
}

#[test]
fn a_link_carries_its_href_and_navigates_when_pressed() {
	let router = Router::<Screen>::new(Screen::Home);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&router.link(note(7), "Open note 7"));
	let link = surface.root().unwrap();

	let props = surface.props(link);
	assert_eq!(props.href(), Some("/notes/7"));
	assert_eq!(props.accessibility_label.as_deref(), Some("Open note 7"));
	assert_eq!(Screen::parse("/notes/7"), Ok(note(7)));

	surface.dispatch_event(link, &NativeEvent::Press);
	assert_eq!(router.current(), note(7));
	assert!(router.can_go_back());
}

#[test]
fn the_router_renders_the_current_route() {
	let router = Router::<Screen>::new(Screen::Home);
	assert_eq!(router.render().text_content(), "Home");

	router.navigate(note(3));
	assert_eq!(router.render().text_content(), "Note 3");
}

#[test]
fn system_back_follows_guest_history_and_root_availability() {
	let router = Router::<Screen>::new(Screen::Home);
	assert_eq!(router.render().props.can_go_back, None);
	router.navigate(note(7));
	router.navigate(Screen::Settings);
	let mut surface = NativeSurface::new(HeadlessBackend::default());
	for previous in [note(7), Screen::Home] {
		let screen = router.render();
		assert_eq!(screen.props.can_go_back, Some(true));
		surface.set_root(&screen);
		let root = surface.root().unwrap();
		surface.dispatch_event(
			root,
			&NativeEvent::Key(leptoskit_native::KeyPress::new("BrowserBack").alt()),
		);
		assert_ne!(router.current(), previous);
		surface.dispatch_event(
			root,
			&NativeEvent::Key(leptoskit_native::KeyPress::new("BrowserBack")),
		);
		assert_eq!(router.current(), previous);
	}
	let screen = router.render();
	assert_eq!(screen.props.can_go_back, None);
	surface.set_root(&screen);
	surface.dispatch_event(
		surface.root().unwrap(),
		&NativeEvent::Key(leptoskit_native::KeyPress::new("BrowserBack")),
	);
	assert_eq!(router.current(), Screen::Home);
}
