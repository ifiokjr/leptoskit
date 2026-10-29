#![cfg(all(feature = "rive", not(target_arch = "wasm32")))]

use std::fmt::Write as _;
use std::time::Duration;

use leptoskit_native::RiveEnvironment;
use leptoskit_native::RiveError;
use leptoskit_native::RiveFrame;
use leptoskit_native::RivePresenter;
use leptoskit_rive::Asset;
use leptoskit_rive::Name;
use leptoskit_rive::Playback;
use leptoskit_rive::Scene;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Style;
use serde_json::Value;

fn content() -> Value {
	serde_json::json!({
		"rive": Playback::new(Asset::new(include_bytes!("../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv")).unwrap())
			.scene(Scene::StateMachine(Name::new("Machine").unwrap())),
		"accessibility_label": "Triangle",
		"style": Style { width: Dimension::Points(128.0), height: Dimension::Points(128.0), ..Style::default() },
	})
}

fn guest(kind: i32, props: &[Value]) -> String {
	let mut data = String::new();
	let mut calls = String::new();
	let mut offset = 0;
	for props in props {
		let bytes = serde_json::to_vec(props).unwrap();
		let mut escaped = String::with_capacity(bytes.len() * 3);
		for byte in &bytes {
			write!(&mut escaped, "\\{byte:02x}").unwrap();
		}
		write!(&mut data, "(data (i32.const {offset}) \"{escaped}\")").unwrap();
		write!(
			&mut calls,
			"(call $props (i64.const 1) (i32.const {offset}) (i32.const {}))",
			bytes.len()
		)
		.unwrap();
		offset += bytes.len();
	}
	let pages = offset.div_ceil(65536);
	format!(
		r#"(module
        (import "leptoskit" "node_create" (func $create (param i64 i32)))
        (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
        (import "leptoskit" "commit" (func $commit))
        (memory (export "memory") {pages}) {data}
        (func (export "leptoskit_main") (call $create (i64.const 1) (i32.const {kind})) {calls} (call $commit))
        (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) unreachable))"#
	)
}

#[derive(Default)]
struct Image(Vec<u8>);
impl RivePresenter for Image {
	fn visible(&mut self, _id: NodeId) -> bool {
		true
	}

	fn present(&mut self, _id: NodeId, frame: &RiveFrame<'_>) -> Result<(), RiveError> {
		self.0 = frame.bytes().to_vec();
		Ok(())
	}
}

#[test]
fn vector_keeps_its_guest_code_separate_from_rive() {
	let vector = leptoskit_native::Element::vector(
		leptoskit_native::ViewBox::new(0.0, 0.0, 128.0, 128.0).unwrap(),
	)
	.aria_label("Static drawing");
	let props = serde_json::to_value(vector.props).unwrap();
	let mut host = WasmHost::headless(guest(15, &[props]).as_bytes()).unwrap();

	host.start(128.0, 128.0).unwrap();
	let root = host.surface().root().unwrap();
	assert_eq!(
		host.surface().kind(root),
		leptoskit_native::NativeKind::Vector
	);
	assert!(host.surface().props(root).drawing.is_some());
	assert!(!host.has_rive_players());
}

#[test]
fn a_replacement_guest_renders_its_own_scene_on_the_retained_widget() {
	let mut host = WasmHost::headless(guest(16, &[content()]).as_bytes()).unwrap();
	host.start(128.0, 128.0).unwrap();
	let root = host.surface().root().unwrap();
	let identity = host.identity();
	let mut replacement = content();
	replacement["accessibility_label"] = serde_json::json!("Replacement triangle");

	host.replace_guest(guest(16, &[replacement]).as_bytes())
		.unwrap();
	assert_ne!(host.identity(), identity);
	host.start(128.0, 128.0).unwrap();
	assert_eq!(host.surface().root(), Some(root));
	assert_eq!(
		host.surface().props(root).accessibility_label.as_deref(),
		Some("Replacement triangle")
	);
	let mut image = Image::default();
	host.tick_rive_with(Duration::ZERO, RiveEnvironment::default(), &mut image)
		.unwrap();
	assert_eq!(image.0.len(), 128 * 128 * 4);
	assert!(image.0.chunks_exact(4).any(|pixel| pixel[3] != 0));
}

#[test]
fn a_replacement_guest_cannot_reuse_the_previous_guests_scene_by_node_id() {
	let mut host = WasmHost::headless(guest(16, &[content()]).as_bytes()).unwrap();
	host.start(128.0, 128.0).unwrap();
	assert!(host.has_rive_players());
	let replacement = guest(16, &[serde_json::json!({"rive_inputs": {}})]);

	host.replace_guest(replacement.as_bytes()).unwrap();
	assert!(
		host.start(128.0, 128.0)
			.unwrap_err()
			.to_string()
			.contains("rive_inputs")
	);
}

#[test]
fn compact_input_updates_retain_a_node_scoped_asset_and_render_it() {
	let mut update = content();
	update.as_object_mut().unwrap().remove("rive");
	update["rive_inputs"] =
		serde_json::json!({"amount": {"value": {"Number": 0.5}, "sequence": 0}});
	let wasm = guest(16, &[content(), update]);
	let mut host = WasmHost::headless(wasm.as_bytes()).unwrap();
	host.start(128.0, 128.0).unwrap();
	assert!(host.has_rive_players());
	let root = host.surface().root().unwrap();
	assert_eq!(
		host.surface().kind(root),
		leptoskit_native::NativeKind::Rive
	);
	assert_eq!(
		host.surface()
			.props(root)
			.rive
			.as_ref()
			.unwrap()
			.inputs()
			.len(),
		1
	);
	let mut image = Image::default();
	assert!(
		!host
			.tick_rive_with(Duration::ZERO, RiveEnvironment::default(), &mut image)
			.unwrap()
	);
	assert_eq!(image.0.len(), 128 * 128 * 4);
	assert!(image.0.chunks_exact(4).any(|pixel| pixel[3] != 0));
}

#[test]
fn compact_updates_cannot_select_another_nodes_asset_or_bypass_binding_limits() {
	let update = serde_json::json!({"rive_inputs": {}});
	for (kind, props) in [
		(16, vec![update.clone()]),
		(0, vec![content(), update.clone()]),
	] {
		let mut host = WasmHost::headless(guest(kind, &props).as_bytes()).unwrap();
		assert!(
			host.start(128.0, 128.0)
				.unwrap_err()
				.to_string()
				.contains("rive_inputs")
		);
	}
	let mut ambiguous = content();
	ambiguous["rive_inputs"] = serde_json::json!({});
	let mut host = WasmHost::headless(guest(16, &[ambiguous]).as_bytes()).unwrap();
	assert!(host.start(128.0, 128.0).is_err());
	let inputs = (0..65)
		.map(|index| {
			(
				format!("input{index}"),
				serde_json::json!({"value": "Trigger", "sequence": 0}),
			)
		})
		.collect::<serde_json::Map<_, _>>();
	let mut host = WasmHost::headless(
		guest(16, &[content(), serde_json::json!({"rive_inputs": inputs})]).as_bytes(),
	)
	.unwrap();
	assert!(
		host.start(128.0, 128.0)
			.unwrap_err()
			.to_string()
			.contains("64 input")
	);
}

#[test]
fn compact_updates_cannot_hide_retained_scene_bytes_from_the_property_limit() {
	let mut initial = content();
	initial["padding"] = serde_json::json!("x".repeat(60000));
	let update = serde_json::json!({"rive_inputs": {}, "text": "x".repeat(6000)});
	let mut host = WasmHost::headless(guest(16, &[initial, update]).as_bytes()).unwrap();
	assert!(
		host.start(128.0, 128.0)
			.unwrap_err()
			.to_string()
			.contains("render property limit exceeded by retained Rive scene")
	);
	assert!(host.surface().root().is_none());
	assert!(host.render_error().is_some());
	assert!(!host.rive_needs_frame());
	assert!(
		!host
			.tick_rive(Duration::ZERO, RiveEnvironment::default())
			.unwrap()
	);
	let mut image = Image::default();
	assert!(
		!host
			.tick_rive_with(Duration::ZERO, RiveEnvironment::default(), &mut image)
			.unwrap()
	);
	assert!(image.0.is_empty());
}

#[test]
fn compact_inputs_require_an_installed_scene_and_cannot_accompany_a_replacement() {
	let label = serde_json::json!({"accessibility_label": "Triangle"});
	let compact = serde_json::json!({"rive_inputs": {}});
	for (kind, props, diagnostic) in [
		(
			0,
			vec![label.clone(), compact.clone()],
			"requires an existing Rive node",
		),
		(
			16,
			vec![label, compact.clone()],
			"no previously installed asset",
		),
		(
			16,
			vec![{
				let mut replacement = content();
				replacement["rive_inputs"] = serde_json::json!({});
				replacement
			}],
			"no replacement asset",
		),
	] {
		let mut host = WasmHost::headless(guest(kind, &props).as_bytes()).unwrap();
		assert!(
			host.start(128.0, 128.0)
				.unwrap_err()
				.to_string()
				.contains(diagnostic)
		);
		assert!(host.surface().root().is_none());
	}
}

#[test]
fn repeated_compact_updates_release_the_previous_update_payload() {
	let mut initial = content();
	initial["padding"] = serde_json::json!("x".repeat(1000));
	let update = serde_json::json!({
		"rive_inputs": {},
		"text": "x".repeat(1000),
		"accessibility_label": "Triangle",
	});
	let mut props = vec![initial];
	props.extend(std::iter::repeat_n(update, 100));
	let mut host = WasmHost::headless(guest(16, &props).as_bytes()).unwrap();
	host.start(128.0, 128.0).unwrap();
	assert!(host.has_rive_players());
}

#[test]
fn compact_updates_cannot_return_the_retained_scene_budget_to_other_nodes() {
	let mut initial = content();
	initial["padding"] = serde_json::json!("");
	let padding = 65510 - serde_json::to_vec(&initial).unwrap().len();
	initial["padding"] = serde_json::json!("x".repeat(padding));
	let initial = initial.to_string().replace('"', "\\22");
	let extra = serde_json::json!({"text": "x".repeat(1000)}).to_string();
	let extra_len = extra.len();
	let extra = extra.replace('"', "\\22");
	let wasm = format!(
		r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
 (memory (export "memory") 2)
 (global $id (mut i64) (i64.const 1))
 (data (i32.const 0) "{initial}")
 (data (i32.const 65510) "{{\22rive_inputs\22:{{}}}}")
 (data (i32.const 65536) "{extra}")
 (func (export "leptoskit_main")
   (loop $build
     (call $create (global.get $id) (i32.const 16))
     (call $props (global.get $id) (i32.const 0) (i32.const 65510))
     (call $props (global.get $id) (i32.const 65510) (i32.const 18))
     (global.set $id (i64.add (global.get $id) (i64.const 1)))
     (br_if $build (i64.le_u (global.get $id) (i64.const 64)))))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
   (call $props (i64.const 65) (i32.const 65536) (i32.const {extra_len}))
   i32.const 0))"#
	);
	let mut host = WasmHost::headless(wasm.as_bytes()).unwrap();
	host.start(128.0, 128.0).unwrap();
	let error = host
		.dispatch_wire(1, leptoskit_wasm::EventKind::Press, None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("render retained property limit"), "{error}");
}
