//! The state panel: signals the app names, readable and editable.

use std::rc::Rc;

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Value;
use reactive_graph::effect::ImmediateEffect;
use reactive_graph::owner::on_cleanup;
use reactive_graph::traits::Set;
use reactive_graph::traits::Track;
use reactive_graph::traits::WithUntracked;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::registry;
use crate::registry::Report;
use crate::registry::Slot;
use crate::registry::Source;
use crate::value::encode;
use crate::value::type_name;

/// Show `signal` in the `DevTools` as `name`, and let them edit it.
///
/// Only state named this way appears: the `DevTools` can't see the app's
/// other signals, so naming one is how it opts in. The state panel shows the
/// value as JSON with its Rust type, updated each time the signal changes,
/// and an edit sets the signal to the JSON decoded back into `T`. A value
/// over 64 KiB is shown as a size only, and can't be edited.
///
/// Exposing another signal under the same name replaces the first. A signal
/// exposed inside a component leaves the panel when the component
/// unmounts.
///
/// ```ignore
/// let count = RwSignal::new(0_usize);
/// #[cfg(feature = "devtools")]
/// leptoskit::devtools::expose("cart.count", count);
/// ```
///
/// Outside a `leptoskit dev` build this does nothing.
pub fn expose<S, T>(name: &str, signal: S)
where
	S: Track + WithUntracked<Value = T> + Set<Value = T> + Clone + Send + Sync + 'static,
	T: Serialize + DeserializeOwned + 'static,
{
	if !registry::active() {
		return;
	}

	let id = registry::register(
		Some(Slot::State(name.to_owned())),
		Rc::new(StateSource {
			name: name.to_owned(),
			signal: signal.clone(),
		}),
	);
	// Tracking the signal is all the app's own writes pay for; the value is
	// serialized in the agent's flush.
	let effect = ImmediateEffect::new_isomorphic(move || {
		signal.track();
		registry::mark_dirty(id);
	});
	registry::keep(id, effect);
	on_cleanup(move || registry::unregister(id));
}

struct StateSource<S> {
	name: String,
	signal: S,
}

impl<S, T> StateSource<S>
where
	S: WithUntracked<Value = T> + Set<Value = T>,
	T: DeserializeOwned,
{
	fn set(&self, value: &Value) -> Result<(), String> {
		if value.truncated {
			return Err(format!(
				"the new value of {:?} arrived truncated, so it wasn't set",
				self.name
			));
		}
		let decoded: T = serde_json::from_value(value.json.clone())
			.map_err(|error| format!("{:?} holds {}: {error}", self.name, type_name::<T>()))?;
		match self.signal.try_set(decoded) {
			None => Ok(()),
			Some(_) => Err(format!("{:?} is no longer mounted", self.name)),
		}
	}
}

impl<S, T> Source for StateSource<S>
where
	S: WithUntracked<Value = T> + Set<Value = T> + 'static,
	T: Serialize + DeserializeOwned + 'static,
{
	fn collect(&self, report: &mut Report) {
		// A disposed signal reports nothing, so its state leaves the panel.
		if let Some(value) = self.signal.try_with_untracked(encode) {
			report.state(self.name.clone(), value);
		}
	}

	fn apply(&self, command: &Command) -> Option<Result<(), String>> {
		match command {
			Command::SetState { name, value } if *name == self.name => Some(self.set(value)),
			_ => None,
		}
	}
}
