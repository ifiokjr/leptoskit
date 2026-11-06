//! Compiled guest fixture rendering an agent's document inside the native
//! interpreter.
//!
//! The document arrives as text at runtime, as it would from an agent. Its
//! stepper and slider write variables; the text that reads them must follow
//! through the guest's region refresh, the native slider must stay the same
//! widget while it moves, its card must follow the platform's dark
//! appearance, and its button's action must reach the application's handler.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use leptoskit::components::Text;
use leptoskit::components::View;
use leptoskit::view;
use leptoskit_agent_ui::AgentUi;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

/// What the agent sent.
const DOCUMENT: &str = "<Var name=\"rent\" value={1800} />
<Var name=\"saving\" value={500} />

<Stepper bind=\"rent\" step={100} label=\"Rent\" format=\"money\" />

<Slider bind=\"saving\" min={0} max={2000} step={50} label=\"Saving\" format=\"money\" />

<Card title=\"Plan\">

Left: {money(income - rent)}, saving {money(saving)}

</Card>

<Button action=\"apply\" value={rent}>Apply</Button>
";

/// Mounts the application's own text above the agent's document.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let owner = Owner::new();
	let (answer, applied) = owner.with(|| {
		(
			RwSignal::new(DOCUMENT.to_owned()),
			RwSignal::new(String::from("Nothing applied")),
		)
	});
	let ui = AgentUi::new()
		.data("income", 4200)
		.on_action(move |action| {
			applied.set(format!("Applied {} {}", action.name, action.value));
		});

	leptoskit::guest::mount(move || {
		let _scope = &owner;
		let document = ui.view(move || answer.get());
		view! {
			<View>
				<Text>{applied.get()}</Text>
				{document}
			</View>
		}
	});
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the interpreter's event payload buffer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI this guest implements.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
