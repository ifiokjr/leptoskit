//! Deterministic widget tests over the production renderer and layout engine.
//!
//! [`WidgetTester`] owns a render closure and a surface. Actions dispatch
//! through the renderer's event handlers and pump the resulting tree. They
//! simulate widget events; they do not inject operating-system input.
//!
//! [`Queries`] adds Testing Library style assertions over the same
//! [`Finder`]s: `get_by_text`, `query_by_label`, `get_all_by_role`, and so
//! on. The browser crate `leptoskit_testing_library` implements it for a
//! mounted web screen, so a test reads the same on both.
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_testing::Finder;
//! use leptoskit_testing::Queries;
//! use leptoskit_testing::WidgetTester;
//! let tester = WidgetTester::new(|| Element::text("Hello"), 390.0, 844.0)?;
//! tester.find(&Finder::text("Hello"))?;
//! tester.get_by_text("Hello")?;
//! assert!(tester.query_by_text("Goodbye")?.is_none());
//! # Ok::<(), leptoskit_testing::TestError>(())
//! ```

use std::fmt::Write;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::Frame;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::Role;
use leptoskit_yoga::NodeId;
pub use queries::Queries;

pub mod capture;
#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
pub mod control;
#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
pub mod native;
mod queries;
#[cfg(all(feature = "native-ui", not(target_arch = "wasm32")))]
pub(crate) mod webdriver;

/// A semantic query, re-evaluated against the current committed tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finder {
	/// Exact visible text property (not an ancestor's aggregated descendants).
	Text(String),
	/// Exact accessibility label.
	Label(String),
	/// A kind and accessible name; the text is used when no label is set.
	Role(NativeKind, String),
	/// An ARIA role, explicit or implied by the widget kind, and an accessible
	/// name; the text is used when no label is set.
	AriaRole(Role, String),
	/// A stable identifier set with `Element::test_id`.
	TestId(String),
}

impl Finder {
	/// Finds exact text.
	pub fn text(text: impl Into<String>) -> Self {
		Self::Text(text.into())
	}

	/// Finds an accessibility label.
	pub fn label(label: impl Into<String>) -> Self {
		Self::Label(label.into())
	}

	/// Finds a widget kind with an accessible name.
	pub fn role(kind: NativeKind, name: impl Into<String>) -> Self {
		Self::Role(kind, name.into())
	}

	/// Finds an ARIA role with an accessible name, like Testing Library's
	/// `getByRole`: the role a widget kind implies counts, so
	/// `aria_role(Role::Button, ..)` finds buttons and containers that
	/// declare the role.
	pub fn aria_role(role: Role, name: impl Into<String>) -> Self {
		Self::AriaRole(role, name.into())
	}

	/// Finds an explicit test identifier.
	pub fn test_id(id: impl Into<String>) -> Self {
		Self::TestId(id.into())
	}

	fn matches(&self, kind: NativeKind, props: &NativeProps) -> bool {
		match self {
			Self::Text(text) => props.text.as_ref() == Some(text),
			Self::Label(label) => props.accessibility_label.as_ref() == Some(label),
			Self::Role(role, name) => {
				kind == *role
					&& props.accessibility_label.as_ref().or(props.text.as_ref()) == Some(name)
			}
			Self::AriaRole(role, name) => {
				// A live region's announced text names it when nothing else
				// does, mirroring what the assistive technology reads.
				let announced = props
					.accessible_name()
					.or(
						(props.aria().effective_live(kind) != leptoskit_native::Live::Off)
							.then_some(props.aria().live_text.as_deref())
							.flatten(),
					);
				props.aria().effective_role(kind) == Some(*role) && announced == Some(name.as_str())
			}
			Self::TestId(id) => props.test_id() == Some(id.as_str()),
		}
	}
}

/// An owned observation; retaining it does not retain a live node handle.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
	/// The committed widget kind.
	pub kind: NativeKind,
	/// The committed properties.
	pub props: NativeProps,
	/// The frame in viewport coordinates.
	pub frame: Frame,
}

/// A failed query or interaction, with the committed tree for diagnosis.
#[derive(Debug, thiserror::Error)]
#[error("{reason}\n{tree}")]
pub struct TestError {
	/// The operation that could not be completed.
	pub reason: String,
	/// The tree at failure time.
	pub tree: String,
}

/// Owns a component's lifecycle, using the real reconciler, layout, and events.
///
/// The closure is called once on construction and once per [`Self::pump`].
/// Async applications should advance their executor before pumping. A finite
/// pump budget is not a wall-clock wait and does not imply network idleness.
pub struct WidgetTester<B: NativeBackend = HeadlessBackend> {
	surface: NativeSurface<B>,
	render: Box<dyn FnMut() -> Element>,
	viewport: (f32, f32),
}

/// The facts one accessibility line is built from.
struct Line<'a> {
	id: NodeId,
	depth: usize,
	kind: NativeKind,
	aria: &'a leptoskit_native::Aria,
	name: Option<&'a str>,
	checked: Option<leptoskit_native::Tristate>,
	value: Option<f64>,
	live: leptoskit_native::Live,
}

/// Whether a node reports any of its states.
fn has_states(aria: &leptoskit_native::Aria, checked: Option<leptoskit_native::Tristate>) -> bool {
	checked.is_some()
		|| aria.pressed.is_some()
		|| aria.selected.is_some()
		|| aria.expanded.is_some()
		|| aria.busy.is_some()
		|| aria.required.is_some()
		|| aria
			.invalid
			.is_some_and(|invalid| invalid != leptoskit_native::Invalid::False)
		|| aria.current.is_some()
		|| aria.modal.is_some()
		|| aria.read_only.is_some()
}

/// Writes the states a node sets, in a stable order, as ` [state=value]`.
fn write_states(
	entry: &mut String,
	aria: &leptoskit_native::Aria,
	checked: Option<leptoskit_native::Tristate>,
) {
	let flag = |value: Option<bool>| value.filter(|value| *value).map(|_| "true".to_owned());
	let text =
		|value: Option<leptoskit_native::Tristate>| value.map(|value| value.as_str().to_owned());

	for (state, value) in [
		("checked", text(checked)),
		("pressed", text(aria.pressed)),
		("selected", flag(aria.selected)),
		("expanded", flag(aria.expanded)),
		("busy", flag(aria.busy)),
		("modal", flag(aria.modal)),
		("required", flag(aria.required)),
		("read-only", flag(aria.read_only)),
		(
			"invalid",
			aria.invalid
				.filter(|invalid| *invalid != leptoskit_native::Invalid::False)
				.map(|invalid| invalid.as_str().to_owned()),
		),
		(
			"current",
			aria.current
				.filter(|current| *current != leptoskit_native::Current::False)
				.map(|current| current.as_str().to_owned()),
		),
	] {
		if let Some(value) = value {
			write!(entry, " [{state}={value}]").expect("writing to a string");
		}
	}
}

/// Whether a frame is finite and overlaps the viewport at all.
fn within_viewport(frame: Frame, viewport: (f32, f32)) -> bool {
	let finite = [frame.x, frame.y, frame.width, frame.height]
		.into_iter()
		.all(f32::is_finite);

	finite
		&& frame.width > 0.0
		&& frame.height > 0.0
		&& frame.x < viewport.0
		&& frame.y < viewport.1
		&& frame.x + frame.width > 0.0
		&& frame.y + frame.height > 0.0
}

/// Whether a node hides itself and its subtree: a hidden node, a stopped
/// activity indicator, or a transparent one.
fn hides(props: &NativeProps, kind: NativeKind) -> bool {
	props.hidden == Some(true)
		|| (kind == NativeKind::ActivityIndicator && props.running == Some(false))
		|| props
			.opacity
			.is_some_and(|opacity| !opacity.is_finite() || opacity <= 0.0)
}

/// Whether a node clips its subtree to its own frame.
fn clips(props: &NativeProps, kind: NativeKind) -> bool {
	props.clip == Some(true) || kind == NativeKind::ScrollView
}

/// The overlap of two frames.
fn intersect(frame: Frame, clip: Frame) -> Frame {
	let right = (frame.x + frame.width).min(clip.x + clip.width);
	let bottom = (frame.y + frame.height).min(clip.y + clip.height);
	let x = frame.x.max(clip.x);
	let y = frame.y.max(clip.y);

	Frame {
		x,
		y,
		width: right - x,
		height: bottom - y,
	}
}

impl WidgetTester<HeadlessBackend> {
	/// Mounts a widget with deterministic approximate text metrics.
	pub fn new(
		render: impl FnMut() -> Element + 'static,
		width: f32,
		height: f32,
	) -> Result<Self, TestError> {
		Self::with_backend(HeadlessBackend::new(), render, width, height)
	}
}

impl<B: NativeBackend> WidgetTester<B> {
	/// Mounts into a supplied backend, including a real platform backend.
	///
	/// The caller must obey that backend's thread and lifecycle requirements.
	pub fn with_backend(
		backend: B,
		render: impl FnMut() -> Element + 'static,
		width: f32,
		height: f32,
	) -> Result<Self, TestError> {
		let mut tester = Self {
			surface: NativeSurface::new(backend),
			render: Box::new(render),
			viewport: (width, height),
		};
		tester.resize(width, height)?;
		tester.pump();
		Ok(tester)
	}

	/// Commits one render and layout pass. No sleeping or implicit executor.
	pub fn pump(&mut self) {
		self.surface.set_root(&(self.render)());
	}

	/// Re-lays out at a new positive, finite viewport size.
	pub fn resize(&mut self, width: f32, height: f32) -> Result<(), TestError> {
		if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
			return Err(self.error("viewport dimensions must be finite and positive"));
		}

		self.viewport = (width, height);
		self.surface.set_viewport(width, height);
		Ok(())
	}

	/// Read-only access for backend-specific assertions and screenshots.
	pub fn surface(&self) -> &NativeSurface<B> {
		&self.surface
	}

	/// Renders colored layout bounds for diagnosis, not native pixels.
	///
	/// The returned image is explicitly marked as a layout diagnostic. To
	/// capture fonts, images, compositing, and OS dialogs, use the native
	/// tester's `screenshot` method instead.
	pub fn layout_image(&self) -> Result<capture::LayoutImage, capture::CaptureError> {
		let mut frames = Vec::new();
		let mut pending = self.surface.root().into_iter().collect::<Vec<_>>();

		while let Some(id) = pending.pop() {
			if let Some(frame) = self.visible_frame(id) {
				frames.push(frame);
			}

			pending.extend(self.surface.children(id).iter().rev().copied());
		}

		capture::LayoutImage::render(self.viewport.0, self.viewport.1, &frames)
	}

	/// Finds exactly one visible widget; ambiguity is always an error. The
	/// same as [`Queries::get`].
	pub fn find(&self, finder: &Finder) -> Result<Widget, TestError> {
		self.get(finder)
	}

	/// All visible matches, in tree order. Hidden and removed nodes are
	/// excluded.
	pub fn find_all(&self, finder: &Finder) -> Vec<Widget> {
		self.matches(finder)
			.into_iter()
			.map(|id| self.widget(id))
			.collect()
	}

	/// Checks immediately, then pumps at most `max_pumps` until exactly one
	/// visible match exists. Duplicate matches fail immediately.
	pub fn pump_until(&mut self, finder: &Finder, max_pumps: usize) -> Result<Widget, TestError> {
		for step in 0..=max_pumps {
			match self.matches(finder).len() {
				1 => return self.find(finder),

				0 if step < max_pumps => self.pump(),
				0 => break,
				_ => return Err(self.error(format!("ambiguous finder {finder:?}"))),
			}
		}

		Err(self.error(format!(
			"{finder:?} did not appear within {max_pumps} pumps"
		)))
	}

	/// Taps the center of a visible, enabled widget and pumps once.
	///
	/// A different widget covering its center is an error. Descendants can
	/// receive the press and bubble it to the selected widget. Switches and
	/// checkboxes propose their inverse checked state; radios propose
	/// selection. These are semantic events, not operating-system pointer
	/// or keyboard input.
	pub fn tap(&mut self, finder: &Finder) -> Result<(), TestError> {
		let id = self.action_target(finder)?;
		let frame = self.absolute_frame(id);
		let hit = self
			.surface
			.hit_test(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
		let Some(hit) = hit.filter(|&hit| {
			self.visible(hit)
				&& (self.is_descendant(hit, id)
					|| (self.surface.kind(hit) == NativeKind::Pressable
						&& self.is_descendant(id, hit)))
		}) else {
			return Err(self.error(format!("{finder:?} is obscured at its center")));
		};

		if !self.enabled(hit) {
			return Err(self.error(format!("{finder:?} has a disabled hit target")));
		}

		match self.surface.kind(id) {
			NativeKind::Switch | NativeKind::Checkbox => {
				let checked = !self.surface.props(id).checked.unwrap_or(false);
				self.dispatch(id, &NativeEvent::Toggle { checked }, finder)
			}
			NativeKind::Radio => self.dispatch(id, &NativeEvent::Toggle { checked: true }, finder),
			_ => self.dispatch(hit, &NativeEvent::Press, finder),
		}
	}

	/// Holds a visible, enabled `Button` or `Pressable` down without
	/// releasing it, so its `active:` utilities, or the default pressed dim,
	/// apply until [`Self::release`]. Like [`Self::tap`], this is a semantic
	/// hold, not operating-system pointer input, and it runs no handlers.
	pub fn hold(&mut self, finder: &Finder) -> Result<(), TestError> {
		let id = self.action_target(finder)?;

		if !matches!(
			self.surface.kind(id),
			NativeKind::Button | NativeKind::Pressable
		) {
			return Err(self.error(format!("{finder:?} is not a Button or Pressable")));
		}

		self.surface.press_in(id);
		Ok(())
	}

	/// Releases the control [`Self::hold`] holds without activating it; tap
	/// it to model the activation a release usually brings.
	pub fn release(&mut self) -> Result<(), TestError> {
		let Some(id) = self.surface.pressed() else {
			return Err(self.error("no control is held"));
		};

		self.surface.press_out(id);
		Ok(())
	}

	/// Proposes a switch, checkbox, or radio selection and pumps once.
	///
	/// Radio controls can only propose `true`; the owner updates the entire
	/// group's selection. A handler may reject the proposal by retaining state.
	pub fn toggle(&mut self, finder: &Finder, checked: bool) -> Result<(), TestError> {
		let id = self.action_target(finder)?;
		let kind = self.surface.kind(id);

		if !matches!(
			kind,
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
		) {
			return Err(self.error(format!("{finder:?} is not a boolean control")));
		}

		if kind == NativeKind::Radio && !checked {
			return Err(self.error("radio controls only propose selection"));
		}

		self.dispatch(id, &NativeEvent::Toggle { checked }, finder)
	}

	/// Proposes a finite slider value, snapped and clamped to its configured
	/// step grid, and pumps once. This simulates a value event, not a drag.
	pub fn set_slider_value(&mut self, finder: &Finder, value: f64) -> Result<(), TestError> {
		let id = self.action_target(finder)?;

		if self.surface.kind(id) != NativeKind::Slider {
			return Err(self.error(format!("{finder:?} is not a slider")));
		}

		let state = self.surface.props(id).slider.unwrap_or_default();
		let Some(value) = state.proposed_value(value) else {
			return Err(self.error("slider proposals must be finite"));
		};

		self.dispatch(id, &NativeEvent::ValueChange { value }, finder)
	}

	/// Replaces a text input's value through its change handler and pumps once.
	pub fn enter_text(
		&mut self,
		finder: &Finder,
		value: impl Into<String>,
	) -> Result<(), TestError> {
		let id = self.action_target(finder)?;
		if !matches!(
			self.surface.kind(id),
			NativeKind::TextInput | NativeKind::PasswordInput
		) {
			return Err(self.error(format!("{finder:?} is not a text input")));
		}
		self.dispatch(
			id,
			&NativeEvent::Change {
				value: value.into(),
			},
			finder,
		)
	}

	/// Moves focus out of a text input through its blur handler and pumps
	/// once, the way leaving a field does on every backend.
	pub fn blur(&mut self, finder: &Finder) -> Result<(), TestError> {
		let id = self.action_target(finder)?;
		if !matches!(
			self.surface.kind(id),
			NativeKind::TextInput | NativeKind::PasswordInput
		) {
			return Err(self.error(format!("{finder:?} is not a text input")));
		}
		self.dispatch(id, &NativeEvent::Blur, finder)
	}

	/// Dispatches a scroll offset to a scroll view and pumps once.
	///
	/// The component must render the resulting state; this does not emulate a
	/// native scroll view's internal content offset.
	pub fn scroll(&mut self, finder: &Finder, x: f32, y: f32) -> Result<(), TestError> {
		let id = self.action_target(finder)?;

		if self.surface.kind(id) != NativeKind::ScrollView || !x.is_finite() || !y.is_finite() {
			return Err(self.error("scroll requires a scroll view and finite offsets"));
		}

		self.dispatch(
			id,
			&NativeEvent::Scroll {
				offset_x: x,
				offset_y: y,
			},
			finder,
		)
	}

	/// A stable, readable tree, including hidden widgets, without allocation
	/// IDs.
	pub fn tree(&self) -> String {
		fn diagnostic_value(kind: NativeKind, value: Option<&str>) -> Option<&str> {
			if kind == NativeKind::PasswordInput {
				value.map(|_| "<redacted>")
			} else {
				value
			}
		}

		let mut output = String::new();
		let mut pending = self
			.surface
			.root()
			.map(|id| (id, 0))
			.into_iter()
			.collect::<Vec<_>>();

		while let Some((id, depth)) = pending.pop() {
			let props = self.surface.props(id);
			let frame = self.absolute_frame(id);
			let kind = self.surface.kind(id);
			// Writing to a String cannot fail.
			writeln!(
				output,
				"{}{} text={:?} label={:?} value={:?} frame=({:.1},{:.1},{:.1},{:.1}) visible={} \
				 enabled={}",
				"  ".repeat(depth),
				kind,
				diagnostic_value(kind, props.text.as_deref()),
				props.accessibility_label,
				diagnostic_value(kind, props.value.as_deref()),
				frame.x,
				frame.y,
				frame.width,
				frame.height,
				self.visible(id),
				self.enabled(id)
			)
			.expect("writing to a string");
			pending.extend(
				self.surface
					.children(id)
					.iter()
					.rev()
					.map(|&child| (child, depth + 1)),
			);
		}

		output
	}

	/// What assistive technology sees, as an indented outline.
	///
	/// One line per accessibility-relevant element: its role, accessible
	/// name, states, values, and live-region politeness. Elements hidden
	/// with `aria_hidden` disappear with their subtrees, control content
	/// merges into the control, and unnamed plain containers are skipped,
	/// mirroring what a screen reader exposes. Use this for accessibility
	/// assertions and snapshots, like Flutter's semantics tree dump.
	pub fn semantics(&self) -> String {
		let mut lines = Vec::new();

		if let Some(root) = self.surface.root() {
			self.semantics_walk(root, 0, &mut lines);
		}

		let mut output = lines.join("\n");

		if !output.is_empty() {
			output.push('\n');
		}

		output
	}

	fn semantics_walk(&self, id: NodeId, depth: usize, lines: &mut Vec<String>) {
		let props = self.surface.props(id);
		let aria = props.aria();

		if aria.is_hidden() {
			return;
		}

		let kind = self.surface.kind(id);
		let role = aria.effective_role(kind);
		let live = aria.effective_live(kind);
		// The resolver filled `live_text` with what the region announces, so
		// an unnamed live region still shows its read-aloud content.
		let name = props.accessible_name().or(aria.live_text.as_deref());
		let checked = aria.checked.or(match kind {
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio => {
				props.checked.map(leptoskit_native::Tristate::from)
			}
			_ => None,
		});
		let value = aria.value_now.or_else(|| {
			(kind == NativeKind::Slider).then(|| props.slider.unwrap_or_default().value())
		});
		let included = role.is_some()
			|| name.is_some()
			|| aria.description.is_some()
			|| aria.level.is_some()
			|| has_states(aria, checked)
			|| value.is_some()
			|| aria.value_text.is_some()
			|| live != leptoskit_native::Live::Off;

		if included {
			let facts = Line {
				id,
				depth,
				kind,
				aria,
				name,
				checked,
				value,
				live,
			};
			lines.push(self.describe(&facts));
		}

		// A control is one accessibility element with its content merged, and
		// so are a named plain container (like React Native's `accessible`)
		// and a live region, whose announced unit is the whole subtree.
		let merged = role.is_some_and(Role::is_interactive)
			|| live != leptoskit_native::Live::Off
			|| (aria.role.is_none()
				&& name.is_some()
				&& matches!(
					kind,
					NativeKind::View
						| NativeKind::ScrollView
						| NativeKind::Text
						| NativeKind::TextRun
				));

		if !merged {
			for &child in self.surface.children(id) {
				self.semantics_walk(child, depth + usize::from(included), lines);
			}
		}
	}

	/// One node's line: its role, name, states, and value.
	fn describe(&self, line: &Line<'_>) -> String {
		let Line {
			id,
			depth,
			kind,
			aria,
			name,
			checked,
			value,
			live,
		} = *line;
		let mut entry = format!(
			"{}{}",
			"  ".repeat(depth),
			aria.effective_role(kind)
				.map_or(kind.wire_name(), Role::wire_name)
		);

		if let Some(level) = aria.level {
			write!(entry, "({level})").expect("writing to a string");
		}

		if let Some(name) = name {
			write!(entry, " {name:?}").expect("writing to a string");
		}

		if !self.enabled(id) {
			entry.push_str(" [disabled]");
		}

		write_states(&mut entry, aria, checked);

		if let Some(value) = value {
			write!(entry, " value={value}").expect("writing to a string");
		}

		if let Some(text) = &aria.value_text {
			write!(entry, " {text:?}").expect("writing to a string");
		}

		if let Some(description) = &aria.description {
			write!(entry, " description={description:?}").expect("writing to a string");
		}

		if live != leptoskit_native::Live::Off {
			write!(entry, " live={}", live.as_str()).expect("writing to a string");
			let atomic = aria
				.atomic
				.unwrap_or_else(|| aria.effective_role(kind).is_some_and(Role::implicit_atomic));

			if atomic {
				entry.push_str(" atomic");
			}
		}

		entry
	}

	fn error(&self, reason: impl Into<String>) -> TestError {
		TestError {
			reason: reason.into(),
			tree: self.tree(),
		}
	}

	fn dispatch(
		&mut self,
		id: NodeId,
		event: &NativeEvent,
		finder: &Finder,
	) -> Result<(), TestError> {
		if self.surface.dispatch_event(id, event) == EventOutcome::Unhandled {
			if self.surface.kind(id) == NativeKind::PasswordInput {
				return Err(self.error(format!("{finder:?} did not handle a password input event")));
			}

			return Err(self.error(format!("{finder:?} did not handle {event:?}")));
		}

		self.pump();
		Ok(())
	}

	fn action_target(&self, finder: &Finder) -> Result<NodeId, TestError> {
		let id = self.unique(finder)?;

		if !self.enabled(id) {
			return Err(self.error(format!("{finder:?} is disabled")));
		}

		let frame = self.absolute_frame(id);
		let (x, y) = (frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);

		if x < 0.0 || y < 0.0 || x >= self.viewport.0 || y >= self.viewport.1 {
			return Err(self.error(format!("{finder:?} has its center outside the viewport")));
		}

		Ok(id)
	}

	fn unique(&self, finder: &Finder) -> Result<NodeId, TestError> {
		let matches = self.matches(finder);

		match matches.as_slice() {
			[id] => Ok(*id),
			_ => {
				Err(self.error(format!(
					"expected one match for {finder:?}, found {}",
					matches.len()
				)))
			}
		}
	}

	fn matches(&self, finder: &Finder) -> Vec<NodeId> {
		let mut found = Vec::new();
		let mut pending = self.surface.root().into_iter().collect::<Vec<_>>();

		while let Some(id) = pending.pop() {
			if self.visible(id) && finder.matches(self.surface.kind(id), self.surface.props(id)) {
				found.push(id);
			}

			pending.extend(self.surface.children(id).iter().rev().copied());
		}

		found
	}

	fn widget(&self, id: NodeId) -> Widget {
		Widget {
			kind: self.surface.kind(id),
			props: self.surface.props(id).clone(),
			frame: self.absolute_frame(id),
		}
	}

	fn absolute_frame(&self, id: NodeId) -> Frame {
		let mut frame = self.surface.frame(id).expect("reachable nodes have frames");
		let mut parent = self.surface.parent(id);

		while let Some(id) = parent {
			let offset = self
				.surface
				.frame(id)
				.expect("reachable parents have frames");
			frame.x += offset.x;
			frame.y += offset.y;
			parent = self.surface.parent(id);
		}

		frame
	}

	fn visible(&self, id: NodeId) -> bool {
		self.visible_frame(id).is_some()
	}

	fn visible_frame(&self, id: NodeId) -> Option<Frame> {
		let frame = self.absolute_frame(id);

		if !within_viewport(frame, self.viewport) {
			return None;
		}

		self.clip_through_ancestors(id, frame)
	}

	/// Clips a frame by every ancestor that clips, and rejects one that is
	/// hidden, collapsed, or clipped away entirely.
	fn clip_through_ancestors(&self, id: NodeId, frame: Frame) -> Option<Frame> {
		let mut visible = frame;
		let mut current = Some(id);

		while let Some(node) = current {
			let props = self.surface.props(node);
			let kind = self.surface.kind(node);

			if hides(props, kind) {
				return None;
			}

			let layout = self.surface.layout(node);

			if layout.width <= 0.0 || layout.height <= 0.0 {
				return None;
			}

			if node != id && clips(props, kind) {
				visible = intersect(visible, self.absolute_frame(node));

				if visible.width <= 0.0 || visible.height <= 0.0 {
					return None;
				}
			}

			current = self.surface.parent(node);
		}

		Some(visible)
	}

	fn enabled(&self, id: NodeId) -> bool {
		let mut current = Some(id);

		while let Some(node) = current {
			if self.surface.props(node).enabled == Some(false) {
				return false;
			}

			current = self.surface.parent(node);
		}

		true
	}

	fn is_descendant(&self, child: NodeId, ancestor: NodeId) -> bool {
		let mut current = Some(child);

		while let Some(node) = current {
			if node == ancestor {
				return true;
			}

			current = self.surface.parent(node);
		}

		false
	}
}

/// Testing Library queries over the committed tree; failures show
/// [`WidgetTester::tree`].
impl<B: NativeBackend> Queries for WidgetTester<B> {
	type Node = Widget;

	fn query_all(&self, finder: &Finder) -> Vec<Widget> {
		self.find_all(finder)
	}

	fn describe(&self) -> String {
		self.tree()
	}
}
