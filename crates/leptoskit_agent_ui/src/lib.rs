//! <!-- {=agentUiSummary|trim|linePrefix:"//! ":true} -->
//!
//! It interprets interfaces an agent writes as text while the app runs:
//! Markdown with components, variables, formulas, charts, and actions. The
//! documents are data, never code, so a published app shows new interfaces
//! without a new build.
//!
//! <!-- {/agentUiSummary} -->
//!
//! An agent answers in Markdown with components in it, the way MDX mixes
//! them, and [`AgentUi`] turns that text into leptoskit elements on every
//! platform. The agent can lay out text, show figures and charts, declare
//! variables the user changes with steppers and fields, compute with
//! formulas that follow those variables, and offer buttons whose presses
//! come back to the application as [`Action`]s.
//!
//! ```text
//! Here is your month at a glance.
//!
//! <Var name="rent" value={1800} />
//! <Let name="left" value={income - rent - groceries} />
//!
//! <Stat label="Left to spend" value={money(left)} />
//!
//! <Stepper bind="rent" step={50} label="Rent" format="money" />
//!
//! <Chart type="bar" labels={["Rent", "Groceries", "Left"]}
//!        values={[rent, groceries, left]} format="money" />
//!
//! <Button action="apply" value={rent}>Use this rent</Button>
//! ```
//!
//! # Safe by construction
//!
//! The document is data, never code. It can only use the components the
//! application registered, call the built-in formula functions, and write
//! the variables it declared itself. It cannot run code, read the
//! application's state beyond what [`AgentUi::data`] hands it, load
//! images or open links the application has not allowed, or place content
//! outside its own box. Every formula runs on a fixed budget, and every
//! document is held to the [`Limits`]. A button does nothing by itself: its
//! press reaches [`AgentUi::on_action`], and the application decides.
//!
//! # Changing at runtime
//!
//! Nothing here needs a new build of the application. The documents are
//! text that arrives while it runs, so an agent can show something no one
//! planned for. The components are part of the application, so new ones
//! ship with an ordinary code push, and [`AgentUi::reference`] describes
//! every registered component for the agent's instructions, so the agent
//! learns them as soon as the application has them.
//!
//! # Styling
//!
//! Documents style with Tailwind classes and colour roles, such as
//! `bg-surface` and `text-muted`, never colours. Each role reads the
//! application's theme variable and follows the platform's dark appearance,
//! so a document matches the application that shows it; see [`Theme`].
//!
//! Components draw only the widgets the host has: native sliders, switches,
//! and radios where it draws them, and versions built from buttons where it
//! does not; see [`Widgets`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::reactive::For;

mod builtins;
mod chart;
mod class;
mod expr;
mod parse;
mod reference;
mod registry;
mod render;
mod state;
mod theme;
mod value;

pub use expr::FormulaError;
pub use registry::Component;
pub use registry::Context;
pub use registry::Input;
pub use registry::Prop;
pub use registry::PropKind;
pub use registry::Props;
pub use theme::Theme;
pub use theme::Token;
pub use value::Value;

use crate::expr::Budget;
use crate::state::Store;

/// What one document may use, so a document from any agent stays small and
/// fast to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
	/// The longest document, in bytes.
	pub max_source_bytes: usize,
	/// The most elements a document renders. The native host allows 4096 for
	/// the whole screen, so leave room for the application's own.
	pub max_nodes: usize,
	/// How deeply components may nest.
	pub max_depth: usize,
	/// The most variables, inputs and formulas together.
	pub max_variables: usize,
	/// The work one formula may do, in steps.
	pub formula_fuel: usize,
	/// The most items in a list.
	pub max_list: usize,
	/// The largest value an input may hold, in bytes.
	pub max_value_bytes: usize,
	/// The most points a chart draws, across its series.
	pub max_chart_points: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			max_source_bytes: 64 * 1024,
			max_nodes: 1500,
			max_depth: 24,
			max_variables: 128,
			formula_fuel: 20_000,
			max_list: 1000,
			max_value_bytes: 4096,
			max_chart_points: 500,
		}
	}
}

/// Which widgets the host that shows documents can draw.
///
/// Documents only use widgets the host draws, so they never stop an
/// application with an unsupported-widget error. Pick the variant that matches
/// the native shell your users have installed: a code push updates the
/// interface, not the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Widgets {
	/// Every widget, including native sliders, switches, checkboxes, radios,
	/// and spinners: the web, Windows, Linux, the terminal, and macOS, iOS,
	/// and Android apps generated by leptoskit 0.3 or later.
	#[default]
	All,
	/// No native form controls: macOS, iOS, and Android apps generated
	/// before leptoskit 0.3. Sliders draw as a track of buttons, switches and
	/// checkboxes as buttons, radios as a row of buttons, and spinners as
	/// text.
	WithoutControls,
	/// Views, text, buttons, text fields, and images only: HarmonyOS today.
	/// On top of [`WithoutControls`](Self::WithoutControls), line and area
	/// charts draw as columns, pies and donuts as each part's amount and
	/// share, and a `<Pressable>` as a button with its label.
	Basic,
}

impl Widgets {
	/// Whether native form controls draw.
	pub(crate) const fn controls(self) -> bool {
		matches!(self, Self::All)
	}

	/// Whether vector paths and pressables draw.
	pub(crate) const fn vectors(self) -> bool {
		!matches!(self, Self::Basic)
	}
}

/// A button press in a document, for the application to act on.
///
/// The application decides what an action does: send it back to the agent
/// as the user's reply, save a budget, or ask the user to confirm first. The
/// document cannot do any of that itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
	/// The button's `action` attribute, such as `"apply"`.
	pub name: String,
	/// The button's label, as the user saw it.
	pub label: String,
	/// The button's `value` attribute, evaluated when it was pressed.
	pub value: Value,
	/// Every input variable's value: what the user set.
	pub inputs: BTreeMap<String, Value>,
	/// Every variable's value, formulas evaluated and application data
	/// included.
	pub state: BTreeMap<String, Value>,
}

/// A problem in a document, with where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
	/// The line, from 1, or 0 when unknown.
	pub line: usize,
	/// The column, from 1, or 0 when unknown.
	pub column: usize,
	/// What is wrong, written for the agent to fix.
	pub message: String,
}

impl fmt::Display for Diagnostic {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.line == 0 {
			formatter.write_str(&self.message)
		} else {
			write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
		}
	}
}

/// A rendered document.
pub struct Rendered {
	/// The interface.
	pub element: Element,
	/// Problems found, for the agent to fix. The element shows what it could.
	pub diagnostics: Vec<Diagnostic>,
	/// Whether the end of the document was left out because it does not
	/// parse yet, as while the agent is still writing it.
	pub partial: bool,
}

type ActionHandler = dyn Fn(&Action) + Send + Sync;
type ImagePolicy = dyn Fn(&str) -> bool + Send + Sync;

/// Everything a render reads, shared by the closures that re-render parts of
/// a document when its variables change.
pub(crate) struct Inner {
	pub(crate) components: BTreeMap<String, Component>,
	pub(crate) limits: Limits,
	pub(crate) theme: Theme,
	pub(crate) store: Store,
	pub(crate) on_action: Option<Arc<ActionHandler>>,
	pub(crate) images: Option<Arc<ImagePolicy>>,
	pub(crate) show_errors: bool,
	pub(crate) widgets: Widgets,
}

impl Inner {
	pub(crate) const fn budget(&self) -> Budget {
		Budget::new(self.limits.formula_fuel, self.limits.max_list)
	}
}

/// Turns documents an agent writes into leptoskit elements.
///
/// Configure one per conversation or screen: the components it knows, the
/// data it shares with documents, and what happens when the user presses a
/// button. Then [`render`](Self::render) each document, or
/// [`view`](Self::view) one that keeps arriving.
///
/// ```
/// use leptoskit_agent_ui::AgentUi;
///
/// let ui = AgentUi::new()
/// 	.data("income", 5200)
/// 	.on_action(|action| println!("the user chose {}", action.name));
///
/// let rendered =
/// 	ui.render("<Var name=\"rent\" value={1800} />\n\nLeft over: {money(income - rent)}\n");
/// assert!(rendered.diagnostics.is_empty());
/// ```
#[derive(Clone)]
pub struct AgentUi {
	inner: Arc<Inner>,
}

impl Default for AgentUi {
	fn default() -> Self {
		Self::new()
	}
}

impl fmt::Debug for AgentUi {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("AgentUi")
			.field(
				"components",
				&self.inner.components.keys().collect::<Vec<_>>(),
			)
			.field("limits", &self.inner.limits)
			.finish_non_exhaustive()
	}
}

impl AgentUi {
	/// A renderer with the built-in components and default limits.
	pub fn new() -> Self {
		let components = builtins::all()
			.into_iter()
			.map(|component| (component.name.clone(), component))
			.collect();

		Self {
			inner: Arc::new(Inner {
				components,
				limits: Limits::default(),
				theme: Theme::default(),
				store: Store::default(),
				on_action: None,
				images: None,
				show_errors: true,
				widgets: Widgets::All,
			}),
		}
	}

	fn configure(mut self, edit: impl FnOnce(&mut Inner)) -> Self {
		if let Some(inner) = Arc::get_mut(&mut self.inner) {
			edit(inner);
		} else {
			let inner = &*self.inner;
			let mut copy = Inner {
				components: inner.components.clone(),
				limits: inner.limits,
				theme: inner.theme.clone(),
				store: inner.store.clone(),
				on_action: inner.on_action.clone(),
				images: inner.images.clone(),
				show_errors: inner.show_errors,
				widgets: inner.widgets,
			};
			edit(&mut copy);
			self.inner = Arc::new(copy);
		}
		self
	}

	/// Registers a component, replacing a built-in one of the same name.
	///
	/// # Panics
	///
	/// When the name does not start with an uppercase letter: lowercase tags
	/// are not components.
	#[must_use]
	pub fn component(self, component: Component) -> Self {
		assert!(
			component
				.name
				.starts_with(|first: char| first.is_ascii_uppercase()),
			"component names start with an uppercase letter: {}",
			component.name
		);
		self.configure(|inner| {
			inner.components.insert(component.name.clone(), component);
		})
	}

	/// Removes a component, built-in or registered, so documents cannot use
	/// it.
	#[must_use]
	pub fn without(self, name: &str) -> Self {
		self.configure(|inner| {
			inner.components.remove(name);
		})
	}

	/// Replaces the limits.
	#[must_use]
	pub fn limits(self, limits: Limits) -> Self {
		self.configure(|inner| inner.limits = limits)
	}

	/// Replaces the theme: the colour roles documents style with, and the
	/// application theme variables they read.
	#[must_use]
	pub fn theme(self, theme: Theme) -> Self {
		self.configure(|inner| inner.theme = theme)
	}

	/// Handles button presses. Without a handler, buttons that send actions
	/// show as disabled.
	#[must_use]
	pub fn on_action(self, handler: impl Fn(&Action) + Send + Sync + 'static) -> Self {
		self.configure(|inner| inner.on_action = Some(Arc::new(handler)))
	}

	/// Allows images whose source `allow` accepts. Without it, an image
	/// shows its description instead, because loading an address the agent
	/// chose can tell a server that the user is reading.
	#[must_use]
	pub fn allow_images(self, allow: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
		self.configure(|inner| inner.images = Some(Arc::new(allow)))
	}

	/// Whether problems show in place, as small notes where the broken part
	/// would be. On by default; turn it off for people who are not the
	/// document's author. [`Rendered::diagnostics`] lists them either way.
	#[must_use]
	pub fn show_errors(self, show: bool) -> Self {
		self.configure(|inner| inner.show_errors = show)
	}

	/// Which widgets the host draws. [`Widgets::All`] by default; see
	/// [`Widgets`] for the shells that need less.
	#[must_use]
	pub fn widgets(self, widgets: Widgets) -> Self {
		self.configure(|inner| inner.widgets = widgets)
	}

	/// Shares a value with documents as a read-only variable, such as the
	/// month's income or the user's categories as a list of records.
	///
	/// Setting it again updates every formula and chart that reads it.
	#[must_use]
	pub fn data(self, name: &str, value: impl Into<Value>) -> Self {
		self.set_data(name, value);
		self
	}

	/// Updates shared data on a renderer already in use.
	pub fn set_data(&self, name: &str, value: impl Into<Value>) {
		self.inner.store.set_data(name, value.into());
	}

	/// The current value of a variable, formulas evaluated.
	pub fn value(&self, name: &str) -> Option<Value> {
		use crate::expr::Scope;

		let mut budget = self.inner.budget();
		self.inner.store.variable(name, &mut budget).ok().flatten()
	}

	/// The input variables' values: what the user has set.
	pub fn inputs(&self) -> BTreeMap<String, Value> {
		self.inner.store.inputs()
	}

	/// Forgets the documents' variables, keeping the application's data.
	pub fn reset(&self) {
		self.inner.store.clear_document();
	}

	/// Renders a document.
	///
	/// This never fails: a broken part renders as a note, or as nothing when
	/// [`show_errors`](Self::show_errors) is off, and is listed in
	/// [`Rendered::diagnostics`].
	pub fn render(&self, source: &str) -> Rendered {
		render::document(&self.inner, source)
	}

	/// The problems in a document, without keeping what it renders. Send
	/// them back to the agent to fix its document.
	pub fn check(&self, source: &str) -> Vec<Diagnostic> {
		self.render(source).diagnostics
	}

	/// A document that follows `source`, re-rendering when it changes, as
	/// while an agent streams its answer into a signal.
	///
	/// ```
	/// use leptoskit_agent_ui::AgentUi;
	/// use leptoskit_native::reactive::Get;
	/// use leptoskit_native::reactive::RwSignal;
	///
	/// # reactive_graph::owner::Owner::new().with(|| {
	/// let answer = RwSignal::new(String::from("Thinking…"));
	/// let view = AgentUi::new().view(move || answer.get());
	/// # let _ = view;
	/// # });
	/// ```
	pub fn view(&self, source: impl Fn() -> String + Send + Sync + 'static) -> Element {
		let ui = self.clone();
		For(
			move || vec![source()],
			// Reuses the widgets in place as the text grows, rather than
			// replacing the whole document for every streamed word.
			|_| (),
			move |source| ui.render(&source).element,
		)
		.class("flex-col min-w-0")
	}

	/// The language reference for the agent: the syntax, the formula
	/// functions, and every component this renderer knows, with its props.
	///
	/// Put it in the agent's instructions, or in the skill it loads. It
	/// describes the components registered right now, so an application
	/// that adds one in a code push teaches the agent about it with the same
	/// update.
	pub fn reference(&self) -> String {
		reference::write(&self.inner)
	}
}
