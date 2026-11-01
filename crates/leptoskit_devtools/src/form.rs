//! The forms panel: tracked `leptoskit_form` forms, read-only.

use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_devtools_protocol::Command;
use leptoskit_form::FormState;

use crate::registry;
use crate::registry::Report;
use crate::registry::Slot;
use crate::registry::Source;
use crate::value::encode;

/// Show a form's state in the `DevTools` as `name`.
///
/// A [`FormState`] is a plain value with no signal of its own, so call this
/// wherever the app has the latest one: in the closure that renders the
/// form, or after each edit. It costs a comparison while nothing changed,
/// and the agent serializes the form only after the page settles. The forms
/// panel shows the values by field, the errors, the fields the user has
/// left, and whether a submit was attempted. Forms are read-only in the
/// `DevTools`.
///
/// ```ignore
/// #[cfg(feature = "devtools")]
/// leptoskit::devtools::track_form("signup", &state);
/// ```
///
/// Outside a `leptoskit dev` build this does nothing.
pub fn track_form(name: &str, state: &FormState) {
	if !registry::active() {
		return;
	}

	let tracked =
		registry::find::<FormSource>(|key| matches!(key, Slot::Form(form) if form == name));
	if let Some((id, form)) = tracked {
		if form.replace(state) {
			registry::mark_dirty(id);
		}
		return;
	}

	registry::register(
		Some(Slot::Form(name.to_owned())),
		Rc::new(FormSource {
			name: name.to_owned(),
			state: RefCell::new(state.clone()),
		}),
	);
}

struct FormSource {
	name: String,
	state: RefCell<FormState>,
}

impl FormSource {
	/// Keeps `state`, and says whether it differs from the one kept before.
	fn replace(&self, state: &FormState) -> bool {
		let mut current = self.state.borrow_mut();
		if *current == *state {
			return false;
		}
		current.clone_from(state);
		true
	}
}

impl Source for FormSource {
	fn collect(&self, report: &mut Report) {
		report.form(self.name.clone(), encode(&*self.state.borrow()));
	}

	fn apply(&self, _command: &Command) -> Option<Result<(), String>> {
		None
	}
}
