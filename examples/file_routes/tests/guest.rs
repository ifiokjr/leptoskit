//! The built native guest in the interpreter: the router's screens mount and
//! its links navigate.
//!
//! `tests/guest.rs` reads the bundle a native build produced, so run
//! `leptoskit native bundle --target ios` in this directory first; without a
//! bundle the suite reports itself skipped.

use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::props::EventOutcome;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

fn find_text<B: NativeBackend>(
	surface: &NativeSurface<B>,
	node: NodeId,
	wanted: &str,
) -> Option<NodeId> {
	if surface.props(node).text.as_deref() == Some(wanted) {
		return Some(node);
	}
	surface
		.children(node)
		.iter()
		.find_map(|child| find_text(surface, *child, wanted))
}

/// The pressable ancestor of the node whose text reads `label`, where the
/// router listens for presses.
fn pressable_of<B: NativeBackend>(
	surface: &NativeSurface<B>,
	node: NodeId,
	label: &str,
) -> Option<NodeId> {
	let mut current = find_text(surface, node, label)?;
	while surface.kind(current) != NativeKind::Pressable {
		current = surface.parent(current)?;
	}
	Some(current)
}

fn bundled_guest() -> Option<Vec<u8>> {
	let wasm = std::fs::read(".leptoskit/ios/bundle/app.wasm").ok()?;
	println!("running against the built guest bundle");
	Some(wasm)
}

fn mounted_guest(wasm: &[u8]) -> WasmHost<leptoskit_native::HeadlessBackend> {
	let mut host = WasmHost::headless(wasm).expect("the guest compiles and links");
	host.start(390.0, 844.0).expect("the guest mounts");
	host
}

#[test]
fn the_built_guest_mounts_the_today_screen() {
	let Some(wasm) = bundled_guest() else {
		return;
	};
	let host = mounted_guest(&wasm);
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	assert!(
		find_text(surface, root, "Today").is_some(),
		"the Today heading is in the mounted tree"
	);
	assert!(
		find_text(surface, root, "Notes").is_some(),
		"the tab bar is in the mounted tree"
	);
}

#[test]
fn pressing_a_tab_navigates_to_its_screen() {
	let Some(wasm) = bundled_guest() else {
		return;
	};
	let mut host = mounted_guest(&wasm);
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	let tab = pressable_of(surface, root, "Notes").expect("the Notes tab");
	assert_eq!(surface.kind(tab), NativeKind::Pressable);
	let outcome = host
		.dispatch(tab, &NativeEvent::Press)
		.expect("the press dispatches");
	assert_eq!(outcome, EventOutcome::Handled);

	let surface = host.surface();
	let root = surface.root().expect("the root after navigating");
	assert!(
		find_text(surface, root, "Note 2").is_some(),
		"the Notes screen replaced Today"
	);
	assert!(
		find_text(surface, root, "Note 3").is_some(),
		"every note link is on screen"
	);
}
