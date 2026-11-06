//! An agent's document, rendered by a compiled guest in the interpreter,
//! must follow its own variables, keep a moving slider in place, follow dark
//! appearance, and reach the application's handler.
//!
//! This is how a published app shows one: the document is text that arrives
//! at runtime, and the guest that interprets it is the app's ordinary
//! interface bundle, so nothing about the host changes.

use leptoskit_native::Color;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

#[test]
#[ignore = "run test:plugins:guest to compile and load the Rust wasm fixture"]
fn compiled_agent_ui_guest_follows_its_variables_and_sends_actions() {
	let path = std::env::var("LEPTOSKIT_AGENT_UI_GUEST").expect("fixture wasm path");
	let mut host = WasmHost::headless(&std::fs::read(path).expect("fixture wasm"))
		.expect("a guest linking leptoskit_agent_ui instantiates");
	host.start(390.0, 844.0).expect("the agent UI guest mounts");

	let shown = texts(&host);
	assert!(shown.contains(&"Nothing applied".to_owned()), "{shown:?}");
	assert!(shown.contains(&"$1,800".to_owned()), "{shown:?}");
	assert!(
		shown.contains(&"Left: $2,400, saving $500".to_owned()),
		"{shown:?}"
	);

	press(&mut host, "+");
	let shown = texts(&host);
	assert!(shown.contains(&"$1,900".to_owned()), "{shown:?}");
	assert!(
		shown.contains(&"Left: $2,300, saving $500".to_owned()),
		"{shown:?}"
	);

	// The slider is updated in place, not replaced, so a drag in progress
	// keeps going while everything reading the variable follows it.
	let slider = only(&host, NativeKind::Slider);
	host.dispatch(slider, &NativeEvent::ValueChange { value: 750.0 })
		.expect("the slide reaches the guest");
	let shown = texts(&host);
	assert!(
		shown.contains(&"Left: $2,300, saving $750".to_owned()),
		"{shown:?}"
	);
	assert_eq!(only(&host, NativeKind::Slider), slider);
	assert_eq!(
		host.surface()
			.props(slider)
			.slider
			.map(leptoskit_native::SliderState::value),
		Some(750.0)
	);

	// The card's surface colour follows the platform's appearance with no
	// new render from the guest.
	let surface = |host: &WasmHost<HeadlessBackend>| {
		nodes(host)
			.into_iter()
			.filter_map(|node| host.surface().props(node).background)
			.collect::<Vec<_>>()
	};
	assert!(surface(&host).contains(&Color::rgba(128, 128, 128, 24)));
	host.set_dark_mode(true);
	assert!(surface(&host).contains(&Color::rgba(128, 128, 128, 40)));
	assert!(!surface(&host).contains(&Color::rgba(128, 128, 128, 24)));

	press(&mut host, "Apply");
	let shown = texts(&host);
	assert!(
		shown.contains(&"Applied apply 1900".to_owned()),
		"{shown:?}"
	);
}

/// Every node, parents before children.
fn nodes(host: &WasmHost<HeadlessBackend>) -> Vec<NodeId> {
	let surface = host.surface();
	let mut pending = vec![surface.root().expect("a root node")];
	let mut all = Vec::new();
	while let Some(node) = pending.pop() {
		all.push(node);
		pending.extend(surface.children(node).iter().rev());
	}
	all
}

/// The text of every node that shows some, in tree order.
fn texts(host: &WasmHost<HeadlessBackend>) -> Vec<String> {
	let surface = host.surface();
	nodes(host)
		.into_iter()
		.filter_map(|node| surface.props(node).text.clone())
		.collect()
}

/// The one node of `kind`.
fn only(host: &WasmHost<HeadlessBackend>, kind: NativeKind) -> NodeId {
	let mut found = nodes(host)
		.into_iter()
		.filter(|node| host.surface().kind(*node) == kind);
	let node = found.next().unwrap_or_else(|| panic!("a {kind:?}"));
	assert!(found.next().is_none(), "one {kind:?}");
	node
}

/// Presses the button labelled `label`.
fn press(host: &mut WasmHost<HeadlessBackend>, label: &str) {
	let surface = host.surface();
	let button = nodes(host)
		.into_iter()
		.find(|node| {
			surface.kind(*node) == NativeKind::Button
				&& surface.props(*node).text.as_deref() == Some(label)
		})
		.unwrap_or_else(|| panic!("a button labelled {label}"));
	host.dispatch(button, &NativeEvent::Press)
		.expect("the press reaches the guest");
}
