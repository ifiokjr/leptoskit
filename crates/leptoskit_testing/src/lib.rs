//! Deterministic widget tests over the production renderer and layout engine.
//!
//! [`WidgetTester`] owns a render closure and a surface. Actions dispatch
//! through the renderer's event handlers and pump the resulting tree. They
//! simulate widget events; they do not inject operating-system input.
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_testing::Finder;
//! use leptoskit_testing::WidgetTester;
//! let tester = WidgetTester::new(|| Element::text("Hello"), 390.0, 844.0)?;
//! tester.find(&Finder::text("Hello"))?;
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
use leptoskit_native::PropValue;
use leptoskit_yoga::NodeId;

/// A semantic query, re-evaluated against the current committed tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finder {
	/// Exact visible text property (not an ancestor's aggregated descendants).
	Text(String),
	/// Exact accessibility label.
	Label(String),
	/// A kind and accessible name; the text is used when no label is set.
	Role(NativeKind, String),
	/// A stable string in `NativeProps::extra["test_id"]`.
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
			Self::TestId(id) => {
				matches!(props.extra.get("test_id"), Some(PropValue::Str(value)) if value == id)
			}
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

	/// Finds exactly one visible widget; ambiguity is always an error.
	pub fn find(&self, finder: &Finder) -> Result<Widget, TestError> {
		let id = self.unique(finder)?;
		Ok(self.widget(id))
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
		let frame = self.absolute_frame(id);
		if ![frame.x, frame.y, frame.width, frame.height]
			.into_iter()
			.all(f32::is_finite)
			|| frame.width <= 0.0
			|| frame.height <= 0.0
			|| frame.x >= self.viewport.0
			|| frame.y >= self.viewport.1
			|| frame.x + frame.width <= 0.0
			|| frame.y + frame.height <= 0.0
		{
			return false;
		}
		let mut current = Some(id);
		let mut visible = frame;
		while let Some(node) = current {
			let props = self.surface.props(node);
			if props.hidden == Some(true)
				|| (self.surface.kind(node) == NativeKind::ActivityIndicator
					&& props.running == Some(false))
				|| props
					.opacity
					.is_some_and(|opacity| !opacity.is_finite() || opacity <= 0.0)
			{
				return false;
			}
			let layout = self.surface.layout(node);
			if layout.width <= 0.0 || layout.height <= 0.0 {
				return false;
			}
			if node != id
				&& (props.clip == Some(true) || self.surface.kind(node) == NativeKind::ScrollView)
			{
				let clip = self.absolute_frame(node);
				let right = (visible.x + visible.width).min(clip.x + clip.width);
				let bottom = (visible.y + visible.height).min(clip.y + clip.height);
				visible.x = visible.x.max(clip.x);
				visible.y = visible.y.max(clip.y);
				visible.width = right - visible.x;
				visible.height = bottom - visible.y;
				if visible.width <= 0.0 || visible.height <= 0.0 {
					return false;
				}
			}
			current = self.surface.parent(node);
		}
		true
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
