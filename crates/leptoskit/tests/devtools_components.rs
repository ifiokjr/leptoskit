//! `#[component]` names the elements its body creates, for the `DevTools`
//! component view.
#![cfg(feature = "devtools")]

use leptoskit::native::HeadlessBackend;
use leptoskit::native::NativeSurface;
use leptoskit::prelude::*;

#[component]
fn Title(text: &'static str) -> Element {
	Text(text)
}

#[component]
fn ClipCard(quote: &'static str) -> Element {
	// The title is its own component; the quote belongs to the card.
	View().child(Title(quote)).child(Text("from the web"))
}

#[component]
fn Library() -> Element {
	view! {
		<View>
			<ClipCard quote="Asphalt stores the day's heat" />
		</View>
	}
}

#[test]
fn elements_carry_the_component_that_created_them() {
	let card = ClipCard("Asphalt");
	assert_eq!(card.component_name(), Some("ClipCard"));
	assert_eq!(card.children[0].component_name(), Some("Title"));
	assert_eq!(card.children[1].component_name(), Some("ClipCard"));

	// Outside every component, an element has none.
	assert_eq!(View().component_name(), None);
}

#[test]
fn view_markup_names_its_components_too() {
	let library = Library();
	assert_eq!(library.component_name(), Some("Library"));
	let card = &library.children[0];
	assert_eq!(card.component_name(), Some("ClipCard"));
	assert_eq!(card.children[0].component_name(), Some("Title"));
}

#[test]
fn the_devtools_tree_reports_components() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 480.0);
	surface.set_root(&Library());
	let components: Vec<Option<String>> = surface
		.devtools_tree()
		.nodes
		.into_iter()
		.map(|node| node.component)
		.collect();
	assert_eq!(
		components,
		[
			Some("Library".to_owned()),
			Some("ClipCard".to_owned()),
			Some("Title".to_owned()),
			Some("ClipCard".to_owned()),
		]
	);
}
