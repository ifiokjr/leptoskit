//! Components a document can use, and what they receive when they render.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::reactive::Get;

use crate::Action;
use crate::Inner;
use crate::Theme;
use crate::Widgets;
use crate::expr::Scope;
use crate::state::Store;
use crate::state::Variable;
use crate::value::Value;

/// Renders a component from its evaluated props and its rendered children.
pub(crate) type Render = dyn Fn(&Props, &mut Context) -> Result<Element, String> + Send + Sync;

/// A component a document can use by name, such as `<Envelope>`.
///
/// Its description, props, and example are what
/// [`AgentUi::reference`](crate::AgentUi::reference) teaches the agent, so
/// describe it the way you would to a colleague writing the document.
///
/// ```
/// use leptoskit_agent_ui::Component;
/// use leptoskit_agent_ui::Prop;
/// use leptoskit_native::components::Text;
/// use leptoskit_native::components::View;
///
/// let envelope = Component::new(
/// 	"Envelope",
/// 	"A spending envelope: how much of a category's budget is spent.",
/// 	|props, _cx| {
/// 		let name = props.text("name")?;
/// 		let spent = props.number("spent")?;
/// 		let limit = props.number("limit")?;
/// 		Ok(View().child(Text(format!("{name}: {spent} of {limit}"))))
/// 	},
/// )
/// .prop(Prop::text("name", "The category.").required())
/// .prop(Prop::number("spent", "Spent so far this month.").required())
/// .prop(Prop::number("limit", "The month's budget.").required())
/// .example(r#"<Envelope name="Groceries" spent={412} limit={600} />"#);
/// # let _ = envelope;
/// ```
#[derive(Clone)]
pub struct Component {
	pub(crate) name: String,
	pub(crate) summary: String,
	pub(crate) props: Vec<Prop>,
	/// What the content between its tags is for, when it takes any.
	pub(crate) children: Option<String>,
	/// Child tags read as data rather than rendered, such as a chart's
	/// `<Series>`.
	pub(crate) data_children: Vec<String>,
	pub(crate) example: Option<String>,
	/// Whether it draws no control and takes no input, so it may sit inside
	/// a `<Pressable>`.
	pub(crate) inert: bool,
	/// Whether its content must be inert, as a pressable's must.
	pub(crate) inert_content: bool,
	pub(crate) render: Arc<Render>,
}

impl Component {
	/// A component named `name`, which must start with a capital letter.
	///
	/// `render` receives the evaluated props and a [`Context`] holding the
	/// rendered children, and returns the component's element, or a message
	/// the document's author sees in its place.
	pub fn new(
		name: impl Into<String>,
		summary: impl Into<String>,
		render: impl Fn(&Props, &mut Context) -> Result<Element, String> + Send + Sync + 'static,
	) -> Self {
		Self {
			name: name.into(),
			summary: summary.into(),
			props: Vec::new(),
			children: None,
			data_children: Vec::new(),
			example: None,
			inert: false,
			inert_content: false,
			render: Arc::new(render),
		}
	}

	/// Adds a prop.
	#[must_use]
	pub fn prop(mut self, prop: Prop) -> Self {
		self.props.push(prop);
		self
	}

	/// Accepts content between the opening and closing tags; `summary` says
	/// what it is for.
	#[must_use]
	pub fn children(mut self, summary: impl Into<String>) -> Self {
		self.children = Some(summary.into());
		self
	}

	/// Reads child tags named `name` as data, available through
	/// [`Props::data`], instead of rendering them.
	#[must_use]
	pub fn data_child(mut self, name: impl Into<String>) -> Self {
		self.data_children.push(name.into());
		self
	}

	/// An example of the component in use, for the language reference.
	#[must_use]
	pub fn example(mut self, example: impl Into<String>) -> Self {
		self.example = Some(example.into());
		self
	}

	/// Marks the component as drawing no control and taking no input: no
	/// buttons, fields, or scrolling. Only inert components may sit inside a
	/// `<Pressable>`, because the native host rejects a control inside
	/// another.
	#[must_use]
	pub fn inert(mut self) -> Self {
		self.inert = true;
		self
	}

	/// Requires the content between the tags to be inert, as a pressable's
	/// must be.
	#[must_use]
	pub(crate) fn inert_content(mut self) -> Self {
		self.inert_content = true;
		self
	}

	/// The component's tag name.
	pub fn name(&self) -> &str {
		&self.name
	}

	pub(crate) fn prop_spec(&self, name: &str) -> Option<&Prop> {
		self.props.iter().find(|prop| prop.name == name)
	}
}

impl fmt::Debug for Component {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("Component")
			.field("name", &self.name)
			.field("props", &self.props)
			.finish_non_exhaustive()
	}
}

/// What kind of value a prop takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
	/// A number, written `{1800}` or `"1800"`.
	Number,
	/// Text, written `"Groceries"`.
	Text,
	/// `true` or `false`; the bare attribute name means `true`.
	Bool,
	/// A list, written `{[1, 2, 3]}`.
	List,
	/// Any value.
	Any,
	/// The name of an input variable the component reads and writes, written
	/// `"rent"`.
	Variable,
}

impl PropKind {
	const fn label(self) -> &'static str {
		match self {
			Self::Number => "number",
			Self::Text => "text",
			Self::Bool => "boolean",
			Self::List => "list",
			Self::Any => "any",
			Self::Variable => "variable name",
		}
	}
}

/// One prop of a [`Component`].
#[derive(Debug, Clone)]
pub struct Prop {
	pub(crate) name: String,
	pub(crate) kind: PropKind,
	pub(crate) summary: String,
	pub(crate) required: bool,
	pub(crate) default: Option<String>,
	/// For a variable prop: whether the component renders again when the
	/// variable changes.
	pub(crate) follows: bool,
}

impl Prop {
	/// A prop of `kind`.
	pub fn new(name: impl Into<String>, kind: PropKind, summary: impl Into<String>) -> Self {
		Self {
			name: name.into(),
			kind,
			summary: summary.into(),
			required: false,
			default: None,
			follows: true,
		}
	}

	/// A number prop.
	pub fn number(name: impl Into<String>, summary: impl Into<String>) -> Self {
		Self::new(name, PropKind::Number, summary)
	}

	/// A text prop.
	pub fn text(name: impl Into<String>, summary: impl Into<String>) -> Self {
		Self::new(name, PropKind::Text, summary)
	}

	/// A boolean prop.
	pub fn bool(name: impl Into<String>, summary: impl Into<String>) -> Self {
		Self::new(name, PropKind::Bool, summary)
	}

	/// A list prop.
	pub fn list(name: impl Into<String>, summary: impl Into<String>) -> Self {
		Self::new(name, PropKind::List, summary)
	}

	/// A prop of any value.
	pub fn any(name: impl Into<String>, summary: impl Into<String>) -> Self {
		Self::new(name, PropKind::Any, summary)
	}

	/// A prop naming an input variable to read and write.
	pub fn variable(name: impl Into<String>, summary: impl Into<String>) -> Self {
		Self::new(name, PropKind::Variable, summary)
	}

	/// Marks the prop as required.
	#[must_use]
	pub const fn required(mut self) -> Self {
		self.required = true;
		self
	}

	/// For a variable prop: keeps the component from rendering again when
	/// the variable changes, for one that changes it itself as the user
	/// types, such as a text field that would lose focus.
	#[must_use]
	pub const fn unfollowed(mut self) -> Self {
		self.follows = false;
		self
	}

	/// Documents the value used when the prop is left out.
	#[must_use]
	pub fn default(mut self, default: impl Into<String>) -> Self {
		self.default = Some(default.into());
		self
	}

	pub(crate) fn describe(&self) -> String {
		let mut line = format!("`{}` ({})", self.name, self.kind.label());
		if self.required {
			line.push_str(", required");
		}
		if let Some(default) = &self.default {
			line.push_str(", default `");
			line.push_str(default);
			line.push('`');
		}
		line.push_str(": ");
		line.push_str(&self.summary);
		line
	}
}

/// A component's props, evaluated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Props {
	pub(crate) values: BTreeMap<String, Value>,
	/// Child tags read as data, in order.
	pub(crate) data: Vec<(String, Self)>,
}

impl Props {
	/// The prop's value, or [`Value::Null`] when it was left out.
	pub fn get(&self, name: &str) -> &Value {
		static NULL: Value = Value::Null;
		self.values.get(name).unwrap_or(&NULL)
	}

	/// Whether the prop was given.
	pub fn has(&self, name: &str) -> bool {
		self.values.contains_key(name)
	}

	/// A required number.
	pub fn number(&self, name: &str) -> Result<f64, String> {
		match self.get(name) {
			Value::Null => Err(format!("`{name}` is required")),
			value => {
				value
					.as_number()
					.ok_or_else(|| format!("`{name}` must be a number, not {}", value.kind()))
			}
		}
	}

	/// A number, or `default` when it was left out.
	pub fn number_or(&self, name: &str, default: f64) -> Result<f64, String> {
		if self.get(name) == &Value::Null {
			Ok(default)
		} else {
			self.number(name)
		}
	}

	/// Required text; numbers and booleans are written out.
	pub fn text(&self, name: &str) -> Result<String, String> {
		match self.get(name) {
			Value::Null => Err(format!("`{name}` is required")),
			value => Ok(crate::expr::display(value)),
		}
	}

	/// Text, or `default` when it was left out.
	pub fn text_or(&self, name: &str, default: &str) -> String {
		match self.get(name) {
			Value::Null => default.to_owned(),
			value => crate::expr::display(value),
		}
	}

	/// A boolean, or `default` when it was left out.
	pub fn bool_or(&self, name: &str, default: bool) -> bool {
		match self.get(name) {
			Value::Null => default,
			Value::Text(text) => text != "false",
			value => value.truthy(),
		}
	}

	/// A required list.
	pub fn list(&self, name: &str) -> Result<&[Value], String> {
		match self.get(name) {
			Value::List(items) => Ok(items),
			Value::Null => Err(format!("`{name}` is required")),
			value => Err(format!("`{name}` must be a list, not {}", value.kind())),
		}
	}

	/// A required list of numbers.
	pub fn numbers(&self, name: &str) -> Result<Vec<f64>, String> {
		self.list(name)?
			.iter()
			.map(|item| {
				item.as_number()
					.ok_or_else(|| format!("`{name}` must hold numbers, not {}", item.kind()))
			})
			.collect()
	}

	/// A list written out as text, or nothing when it was left out.
	pub fn texts(&self, name: &str) -> Result<Vec<String>, String> {
		match self.get(name) {
			Value::Null => Ok(Vec::new()),
			_ => Ok(self.list(name)?.iter().map(crate::expr::display).collect()),
		}
	}

	/// The child tags named `name` that the component reads as data, such as
	/// a chart's `<Series>`.
	pub fn data<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> + 'a {
		self.data
			.iter()
			.filter(move |(tag, _)| tag == name)
			.map(|(_, props)| props)
	}
}

/// What a component can reach while it renders: its children, the theme,
/// input variables, and actions.
pub struct Context {
	pub(crate) ui: Arc<Inner>,
	pub(crate) children: Vec<Element>,
	/// The content between the tags as plain text, such as a button's label.
	pub(crate) text: String,
	/// Elements the component may still create, to keep the document within
	/// its node limit.
	pub(crate) nodes_left: usize,
}

impl Context {
	/// The rendered content between the component's tags.
	pub fn take_children(&mut self) -> Vec<Element> {
		std::mem::take(&mut self.children)
	}

	/// The content between the component's tags as plain text, without
	/// markup, such as a button's label.
	pub fn text(&self) -> &str {
		&self.text
	}

	/// The colour roles documents draw with.
	pub fn theme(&self) -> &Theme {
		&self.ui.theme
	}

	/// `classes` with colour roles such as `bg-surface` or `text-muted`
	/// replaced by classes that follow the application's theme and dark
	/// appearance, for a component's own styling.
	pub fn style(&self, classes: &str) -> String {
		self.ui.theme.expand(classes)
	}

	/// Which widgets the host draws, for a component that has a fallback.
	pub fn widgets(&self) -> Widgets {
		self.ui.widgets
	}

	/// Claims `count` more elements of the document's node limit, failing
	/// when the document has used them up.
	pub fn reserve(&mut self, count: usize) -> Result<(), String> {
		match self.nodes_left.checked_sub(count) {
			Some(left) => {
				self.nodes_left = left;
				Ok(())
			}
			None => {
				Err(format!(
					"the document is limited to {} elements",
					self.ui.limits.max_nodes
				))
			}
		}
	}

	/// The input variable a [`PropKind::Variable`] prop names, to read and
	/// write.
	pub fn input(&self, props: &Props, prop: &str) -> Result<Input, String> {
		let name = props.text(prop)?;
		match self.ui.store.get(&name) {
			Some(Variable::Input { .. }) => {
				Ok(Input {
					name,
					store: self.ui.store.clone(),
					max_bytes: self.ui.limits.max_value_bytes,
				})
			}
			Some(_) => {
				Err(format!(
					"`{name}` cannot be changed; declare an input with <Var name=\"{name}\" \
					 value={{…}} />"
				))
			}
			None => {
				Err(format!(
					"there is no variable `{name}`; declare it with <Var name=\"{name}\" \
					 value={{…}} />"
				))
			}
		}
	}

	/// A handler that sends `name` and `value` to the application as an
	/// [`Action`], or `None` when the application takes no actions.
	pub fn action(
		&self,
		name: &str,
		value: Value,
		label: &str,
	) -> Option<impl Fn() + Send + Sync + 'static> {
		let handler = self.ui.on_action.clone()?;
		let store = self.ui.store.clone();
		let limits = self.ui.limits;
		let name = name.to_owned();
		let label = label.to_owned();

		Some(move || {
			handler(&Action {
				name: name.clone(),
				label: label.clone(),
				value: value.clone(),
				inputs: store.inputs(),
				state: store.snapshot(limits.formula_fuel, limits.max_list),
			});
		})
	}

	/// Evaluates `name` as a variable, for components that show one.
	pub fn read(&self, name: &str) -> Option<Value> {
		let mut budget = self.ui.budget();
		self.ui.store.variable(name, &mut budget).ok().flatten()
	}
}

/// An input variable a component reads and writes, such as the one a
/// stepper changes.
#[derive(Clone)]
pub struct Input {
	name: String,
	store: Store,
	max_bytes: usize,
}

impl Input {
	/// The variable's name.
	pub fn name(&self) -> &str {
		&self.name
	}

	/// The current value; reading it inside a binding follows it.
	pub fn get(&self) -> Value {
		match self.store.get(&self.name) {
			Some(Variable::Input { signal, .. }) => signal.get(),
			_ => Value::Null,
		}
	}

	/// The current value, without following it: for a field's starting
	/// text.
	pub fn current(&self) -> Value {
		reactive_graph::graph::untrack(|| self.get())
	}

	/// Writes a new value, which every formula and chart reading the
	/// variable follows.
	pub fn set(&self, value: Value) {
		if value.weight() <= self.max_bytes {
			let _ = self.store.write(&self.name, value);
		}
	}
}
