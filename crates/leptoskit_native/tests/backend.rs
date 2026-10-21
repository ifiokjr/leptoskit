//! Mutation batches and the recording backend that external widget hosts
//! consume.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::MutationList;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_yoga::NodeId;

/// Three distinct delete mutations, for widgets of one committed tree.
fn deletes() -> [Mutation; 3] {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().children([View(), View(), View()]));
	let ids: Vec<NodeId> = surface.children(surface.root().unwrap()).to_vec();
	[0, 1, 2].map(|index| Mutation::Delete { id: ids[index] })
}

#[test]
fn mutation_lists_keep_application_order_across_extension() {
	let [a, b, c] = deletes();
	let mut first = MutationList::new();
	assert!(first.is_empty());
	assert_eq!(first.len(), 0);

	first.push(a);
	let mut second = MutationList::new();
	second.push(b);
	second.push(c);
	first.extend(second.clone());

	assert!(!first.is_empty());
	assert_eq!(first.len(), 3);
	assert_eq!(&first.mutations()[1..], second.mutations());
	let owned = first.clone().into_mutations();
	assert_eq!(owned.as_slice(), first.mutations());
}

#[test]
fn taking_the_log_transfers_each_commit_exactly_once() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View().child(Text("Hello")));

	let first = surface.backend_mut().take_log();
	assert!(
		first
			.iter()
			.any(|mutation| matches!(mutation, Mutation::Create { .. }))
	);
	assert!(surface.backend().log().is_empty());
	assert!(surface.backend_mut().take_log().is_empty());

	surface.set_root(&View().child(Text("Changed")));
	let second = surface.backend_mut().take_log();
	assert!(!second.is_empty());
	assert!(
		second
			.iter()
			.all(|mutation| !matches!(mutation, Mutation::Create { .. })),
		"an unchanged structure updates props without recreating widgets: {second:?}"
	);
}

#[test]
fn the_recording_backend_debug_output_names_its_log() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&View());
	let debug = format!("{:?}", surface.backend());

	assert!(debug.starts_with("HeadlessBackend"));
	assert!(debug.contains("log"));
}
