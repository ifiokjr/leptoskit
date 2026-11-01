//! The interface's state, and the actions its controls run.
//!
//! The screens read the [`Model`] through [`read`], so they render again when
//! it changes. Actions update the model and send commands to the hub through
//! [`crate::hub`].

use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::Value;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Update;
use reactive_graph::traits::With;
use reactive_graph::traits::WithUntracked;

use crate::hub;
use crate::model::Model;
use crate::model::Panel;

thread_local! {
	static MODEL: ArcRwSignal<Model> = ArcRwSignal::new(Model::default());
}

/// Reads the model, so the screen renders again when it changes.
pub fn read<T>(read: impl FnOnce(&Model) -> T) -> T {
	MODEL.with(|model| model.with(read))
}

/// Reads the model without subscribing to it.
pub fn peek<T>(read: impl FnOnce(&Model) -> T) -> T {
	MODEL.with(|model| model.with_untracked(read))
}

/// Changes the model.
pub fn update<T>(change: impl FnOnce(&mut Model) -> T) -> T {
	MODEL.with(|model| {
		let mut result = None;
		model.update(|model| result = Some(change(model)));
		result.expect("the update ran")
	})
}

/// Puts a problem on screen.
pub fn problem(message: impl Into<String>) {
	let message = message.into();
	update(|model| model.notice = Some(message));
}

/// Sends a command to the shown session.
fn send(command: Command) {
	if let Some(session) = peek(|model| model.current.clone()) {
		hub::send(&session, command);
	}
}

/// Shows a session.
pub fn show(session: &SessionId) {
	update(|model| model.show(session));
}

/// Selects a node, and selects and highlights it in the app.
pub fn select(node: NodeRef, key: MatchKey) {
	update(|model| model.select(node.clone(), key.clone()));
	send(Command::Select {
		node,
		key: key.clone(),
	});
	send(Command::Highlight { key });
}

/// Shows or hides a node's children.
pub fn toggle(node: &NodeRef) {
	update(|model| model.toggle(node));
}

/// Filters the tree.
pub fn search(text: &str) {
	update(|model| model.search = text.to_owned());
}

/// Turns picking in the app on or off.
pub fn pick() {
	let on = update(|model| {
		model.picking = !model.picking;
		model.picking
	});
	send(Command::Pick { on });
}

/// Asks the shown session for a fresh snapshot.
pub fn refresh() {
	send(Command::RequestSnapshot);
}

/// Shows a panel beside the tree.
pub fn panel(panel: Panel) {
	update(|model| model.panel = panel);
}

/// Edits the selected node's classes before they are applied.
pub fn edit_classes(text: &str) {
	update(|model| model.drafts.classes = Some(text.to_owned()));
}

/// Replaces the selected node's classes in the app.
pub fn apply_classes() {
	let change = peek(|model| {
		let node = model.node()?;
		let draft = model.drafts.classes.clone()?;
		Some((node.id.clone(), draft))
	});

	if let Some((node, draft)) = change {
		let classes = draft.split_whitespace().map(str::to_owned).collect();
		send(Command::SetClasses { node, classes });
	}
}

/// Edits the route to navigate to.
pub fn edit_route(text: &str) {
	update(|model| model.drafts.route = Some(text.to_owned()));
}

/// Navigates the app to the route being edited.
pub fn navigate() {
	let href = peek(|model| model.drafts.route.clone()).unwrap_or_default();
	let href = href.trim();

	if href.is_empty() {
		problem("Enter a route to navigate to, such as /settings.");
		return;
	}

	send(Command::Navigate {
		href: href.to_owned(),
	});
}

/// Refetches a query.
pub fn refetch(key: &str) {
	send(Command::RefetchQuery {
		key: key.to_owned(),
	});
}

/// Marks a query stale.
pub fn invalidate(key: &str) {
	send(Command::InvalidateQuery {
		key: key.to_owned(),
	});
}

/// Edits a query's data, as JSON, before it is saved.
pub fn edit_query(key: &str, json: &str) {
	update(|model| {
		model.drafts.queries.insert(key.to_owned(), json.to_owned());
	});
}

/// Stops editing a query's data.
pub fn cancel_query(key: &str) {
	update(|model| model.drafts.queries.remove(key));
}

/// Replaces a query's data with the JSON being edited.
pub fn save_query(key: &str) {
	let edit = peek(|model| {
		let draft = model.drafts.queries.get(key)?.clone();
		let value = model.session()?.view.queries.get(key)?.clone();
		Some((draft, value))
	});

	if let Some((draft, value)) = edit
		&& let Some(value) = parsed(&draft, &value)
	{
		send(Command::SetQueryData {
			key: key.to_owned(),
			value,
		});
		cancel_query(key);
	}
}

/// Edits named state, as JSON, before it is saved.
pub fn edit_state(name: &str, json: &str) {
	update(|model| {
		model.drafts.state.insert(name.to_owned(), json.to_owned());
	});
}

/// Stops editing named state.
pub fn cancel_state(name: &str) {
	update(|model| model.drafts.state.remove(name));
}

/// Replaces named state with the JSON being edited.
pub fn save_state(name: &str) {
	let edit = peek(|model| {
		let draft = model.drafts.state.get(name)?.clone();
		let value = model.session()?.view.state.get(name)?.clone();
		Some((draft, value))
	});

	if let Some((draft, value)) = edit
		&& let Some(value) = parsed(&draft, &value)
	{
		send(Command::SetState {
			name: name.to_owned(),
			value,
		});
		cancel_state(name);
	}
}

/// The edited JSON as a value of the same type, or a problem on screen.
fn parsed(draft: &str, current: &Value) -> Option<Value> {
	match serde_json::from_str(draft) {
		Ok(json) => {
			Some(Value {
				type_name: current.type_name.clone(),
				json,
				truncated: false,
			})
		}
		Err(error) => {
			problem(format!("That isn't valid JSON: {error}."));
			None
		}
	}
}

/// Starts or stops recording commit timings.
pub fn profile(on: bool) {
	send(if on {
		Command::StartProfiling
	} else {
		Command::StopProfiling
	});
}

/// Highlights the selected node in every session of the app that has it.
pub fn highlight_everywhere() {
	let targets = peek(|model| {
		let key = model.selected.as_ref()?.key.clone();
		let mut sessions: Vec<SessionId> = model.current.iter().cloned().collect();
		sessions.extend(
			model
				.counterparts()
				.iter()
				.filter(|counterpart| counterpart.session.has(Capability::Tree))
				.map(|counterpart| counterpart.session.info.id.clone()),
		);
		Some((key, sessions))
	});

	if let Some((key, sessions)) = targets {
		for session in sessions {
			hub::send(&session, Command::Highlight { key: key.clone() });
		}
	}
}

/// Opens a source location in the editor.
pub fn open_in_editor(url: &str) {
	hub::open(url);
}

/// Clears the problem on screen.
pub fn dismiss() {
	update(|model| model.notice = None);
}
