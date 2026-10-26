//! GTK ownership, mutation application, and controlled native input.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;
use std::rc::Rc;
use std::rc::Weak;
use std::sync::Arc;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Progress;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_native::TextAlign;
use leptoskit_yoga::NodeId;

use crate::frame::Frame;
use crate::measure::GtkTextMeasurer;
use crate::measure::font;

/// Runs on the GTK thread, outside backend borrows. It may update the surface.
pub type EventListener = Rc<dyn Fn(NodeId, NativeEvent)>;

struct Widget {
	kind: NativeKind,
	frame: Frame,
	control: gtk::Widget,
	content: Frame,
	props: NativeProps,
	parent: Option<NodeId>,
	children: Vec<NodeId>,
	rect: graphene::Rect,
	css: gtk::CssProvider,
	css_text: RefCell<String>,
	animation: Option<glib::SourceId>,
	radio_anchor: Option<gtk::CheckButton>,
}

#[derive(Default)]
struct State {
	widgets: HashMap<NodeId, Widget>,
	listener: Option<EventListener>,
	suppressed: Rc<Cell<bool>>,
	dark: Option<bool>,
	hosts: Vec<Host>,
	resize_listener: Option<Rc<dyn Fn(f32, f32)>>,
}

struct Host {
	frame: glib::WeakRef<Frame>,
	root: Option<NodeId>,
}

/// Real GTK4 widgets, created and mutated on the initializing GTK thread.
///
/// Call [`gtk::init`] on the application thread before constructing this
/// backend. The native controls retain their identity during property and
/// layout updates.
pub struct GtkBackend {
	state: Rc<RefCell<State>>,
}

impl std::fmt::Debug for GtkBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("GtkBackend")
			.field("widgets", &self.state.borrow().widgets.len())
			.finish_non_exhaustive()
	}
}

impl Default for GtkBackend {
	fn default() -> Self {
		Self::new()
	}
}

impl GtkBackend {
	/// Creates a backend after GTK initialization on the current thread.
	///
	/// # Panics
	/// Panics if GTK was not initialized on this thread.
	pub fn new() -> Self {
		assert!(
			gtk::is_initialized_main_thread(),
			"initialize GTK on the application thread first"
		);
		Self {
			state: Rc::new(RefCell::new(State::default())),
		}
	}

	/// Replaces the platform event callback.
	pub fn set_event_listener(&self, listener: impl Fn(NodeId, NativeEvent) + 'static) {
		self.state.borrow_mut().listener = Some(Rc::new(listener));
	}

	/// Removes the event callback without changing the widget hierarchy.
	pub fn clear_event_listener(&self) {
		self.state.borrow_mut().listener = None;
	}

	/// Receives window content sizes in GTK logical pixels after allocation.
	/// The callback runs from the `GLib` event loop, outside layout and backend
	/// borrows, so it can update the surface's viewport and reconcile safely.
	pub fn set_resize_listener(&self, listener: impl Fn(f32, f32) + 'static) {
		self.state.borrow_mut().resize_listener = Some(Rc::new(listener));
	}

	/// The zero-minimum outer widget whose bounds are the committed Yoga frame.
	pub fn view(&self, id: NodeId) -> Option<gtk::Widget> {
		self.state
			.borrow()
			.widgets
			.get(&id)
			.map(|widget| widget.frame.clone().upcast())
	}

	/// The native control, for host accessibility integration and native
	/// testing.
	pub fn control(&self, id: NodeId) -> Option<gtk::Widget> {
		self.state
			.borrow()
			.widgets
			.get(&id)
			.map(|widget| widget.control.clone())
	}

	/// Shows the committed root in a desktop window. The host owns its
	/// lifetime.
	pub fn open_window(
		&self,
		root: NodeId,
		title: &str,
		width: i32,
		height: i32,
	) -> Option<gtk::Window> {
		let view = self.view(root)?;

		if view.parent().is_some() {
			return None;
		}
		Some(self.window(Some(root), title, width, height))
	}

	/// Shows a host before a guest has committed its first root.
	/// The first unmounted root committed later attaches to the existing host;
	/// clearing and replacing that root preserves the same native window.
	pub fn create_window(&self, title: &str, width: i32, height: i32) -> gtk::Window {
		let root = self.state.borrow().widgets.iter().find_map(|(id, widget)| {
			(widget.parent.is_none() && widget.frame.parent().is_none()).then_some(*id)
		});
		self.window(root, title, width, height)
	}

	fn window(&self, root: Option<NodeId>, title: &str, width: i32, height: i32) -> gtk::Window {
		let guard = Suppression::new(&self.state);
		let host = Frame::new();
		if let Some(widget) = root.and_then(|root| {
			self.state
				.borrow()
				.widgets
				.get(&root)
				.map(|widget| (widget.frame.clone(), widget.rect))
		}) {
			host.put(&widget.0, widget.1, 0, false);
		}
		self.state.borrow_mut().hosts.push(Host {
			frame: host.downgrade(),
			root,
		});
		let weak = Rc::downgrade(&self.state);
		host.set_resize_listener(move |width, height| {
			let listener = weak
				.upgrade()
				.and_then(|state| state.borrow().resize_listener.clone());
			if let Some(listener) = listener {
				listener(width as f32, height as f32);
			}
		});
		let window = gtk::Window::builder()
			.title(title)
			.default_width(width)
			.default_height(height)
			.child(&host)
			.build();
		window.present();
		self.refresh();
		drop(guard);
		window
	}

	/// Renders a mapped root to a PNG using its actual GTK renderer.
	///
	/// # Errors
	/// Returns a GTK error if no render node or native renderer is available.
	pub fn snapshot_png(&self, root: NodeId) -> Result<Vec<u8>, glib::Error> {
		let widget = self
			.view(root)
			.ok_or_else(|| glib::Error::new(gtk::gio::IOErrorEnum::NotFound, "unknown root"))?;
		let paintable = gtk::WidgetPaintable::new(Some(&widget));
		let snapshot = gtk::Snapshot::new();
		paintable.snapshot(
			&snapshot,
			f64::from(widget.width()),
			f64::from(widget.height()),
		);
		let node = snapshot.to_node().ok_or_else(|| {
			glib::Error::new(
				gtk::gio::IOErrorEnum::Failed,
				"root must be mapped before snapshotting",
			)
		})?;
		let renderer = widget
			.native()
			.and_then(|native| native.renderer())
			.ok_or_else(|| {
				glib::Error::new(gtk::gio::IOErrorEnum::Failed, "root has no renderer")
			})?;
		let viewport = graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
		Ok(renderer
			.render_texture(&node, Some(&viewport))
			.save_to_png_bytes()
			.to_vec())
	}

	fn refresh(&self) {
		Self::refresh_state(&self.state);
	}

	fn refresh_state(shared: &Rc<RefCell<State>>) {
		let guard = Suppression::new(shared);
		let mut state = shared.borrow_mut();
		let mut groups = HashMap::new();

		// GTK4.8's set_group(Some(..)) does not first unlink an existing
		// group. Reconstruct from detached members to avoid linked-list
		// cycles during repeated property commits or group reassignment.
		for widget in state
			.widgets
			.values()
			.filter(|widget| widget.kind == NativeKind::Radio)
		{
			widget
				.control
				.clone()
				.downcast::<gtk::CheckButton>()
				.expect("radio control")
				.set_group(None::<&gtk::CheckButton>);
		}
		for widget in state.widgets.values() {
			if let Some(anchor) = &widget.radio_anchor {
				anchor.set_group(None::<&gtk::CheckButton>);
			}
		}

		for (id, widget) in state
			.widgets
			.iter()
			.filter(|(_, widget)| widget.kind == NativeKind::Radio)
		{
			if let Some(group) = widget.props.radio_group.as_ref() {
				let mut root = *id;
				while let Some(parent) = state.widgets.get(&root).and_then(|widget| widget.parent) {
					root = parent;
				}
				let anchor = groups
					.entry((root, group.name()))
					.or_insert_with(|| widget.radio_anchor.clone().expect("radio anchor"));
				widget
					.control
					.clone()
					.downcast::<gtk::CheckButton>()
					.expect("radio control")
					.set_group(Some(anchor));
			}
		}
		for (root, name) in groups.keys() {
			let mut members = Vec::new();
			collect_visible_group(&state, *root, name, &mut members);

			for (index, id) in members.iter().enumerate() {
				state.widgets[id].control.update_relation(&[
					gtk::accessible::Relation::PosInSet(index as i32 + 1),
					gtk::accessible::Relation::SetSize(members.len() as i32),
				]);
				// GTK4.8 exposes set-position relations internally but omits
				// them from AT-SPI attributes. Preserve the group position in
				// its spoken description on that supported runtime too.
				state.widgets[id].control.update_property(&[
					gtk::accessible::Property::Description(&format!(
						"Option {} of {}",
						index + 1,
						members.len()
					)),
				]);
			}
		}

		for widget in state.widgets.values() {
			configure(widget, state.dark);
		}

		let active = state
			.widgets
			.iter()
			.map(|(id, widget)| {
				let visible = interactive(&state, *id);
				let running = match widget.kind {
					NativeKind::ActivityIndicator => widget.props.running.unwrap_or(true),
					NativeKind::LinearProgressIndicator => {
						widget.props.progress.unwrap_or_default() == Progress::Indeterminate
					}
					_ => false,
				};
				(*id, visible && running && widget.frame.is_mapped())
			})
			.collect::<Vec<_>>();

		for (id, animate) in active {
			let widget = state.widgets.get_mut(&id).expect("collected live widget");

			if let Ok(spinner) = widget.control.clone().downcast::<gtk::Spinner>() {
				spinner.set_spinning(animate);
			}

			if !animate {
				if let Some(source) = widget.animation.take() {
					source.remove();
				}
				continue;
			}

			if widget.kind != NativeKind::LinearProgressIndicator || widget.animation.is_some() {
				continue;
			}
			let weak = widget.control.downgrade();
			widget.animation = Some(glib::timeout_add_local(
				Duration::from_millis(80),
				move || {
					let Some(control) = weak.upgrade().and_downcast::<gtk::ProgressBar>() else {
						return glib::ControlFlow::Break;
					};
					control.pulse();
					glib::ControlFlow::Continue
				},
			));
		}
		drop(state);
		drop(guard);
	}
}

impl Drop for GtkBackend {
	fn drop(&mut self) {
		let guard = Suppression::new(&self.state);
		let mut state = self.state.borrow_mut();
		state.listener = None;
		state.resize_listener = None;

		for widget in state.widgets.values_mut() {
			stop(widget);
		}
		// Hosts may still retain a root. Disconnect callbacks by clearing the
		// weakly referenced ownership map and stop all animations immediately.
		state.widgets.clear();
		drop(state);
		drop(guard);
	}
}

struct Suppression {
	flag: Rc<Cell<bool>>,
	previous: bool,
}

impl Suppression {
	fn new(state: &Rc<RefCell<State>>) -> Self {
		let flag = state.borrow().suppressed.clone();
		let previous = flag.replace(true);
		Self { flag, previous }
	}
}

impl Drop for Suppression {
	fn drop(&mut self) {
		self.flag.set(self.previous);
	}
}

impl NativeBackend for GtkBackend {
	fn apply(&mut self, mutations: &MutationList) {
		let guard = Suppression::new(&self.state);

		for mutation in mutations.mutations() {
			self.apply_one(mutation);
		}
		sync_hosts(&mut self.state.borrow_mut());
		self.refresh();
		drop(guard);
	}

	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		let frame = Frame::new();
		let content = if kind == NativeKind::Pressable {
			Frame::decorative()
		} else {
			Frame::new()
		};
		let control: gtk::Widget = match kind {
			NativeKind::View | NativeKind::Custom(_) => content.clone().upcast(),
			NativeKind::Text | NativeKind::TextRun => {
				gtk::Label::builder()
					.wrap(true)
					.wrap_mode(gtk::pango::WrapMode::WordChar)
					.yalign(0.0)
					.build()
					.upcast()
			}
			NativeKind::Button => gtk::Button::new().upcast(),
			NativeKind::Pressable => {
				let button = gtk::Button::new();
				content.set_can_target(false);
				button.set_child(Some(&content));
				button.upcast()
			}
			NativeKind::TextInput | NativeKind::PasswordInput => {
				let entry = gtk::Entry::new();
				entry.set_visibility(kind != NativeKind::PasswordInput);
				entry.set_input_purpose(if kind == NativeKind::PasswordInput {
					gtk::InputPurpose::Password
				} else {
					gtk::InputPurpose::FreeForm
				});
				entry.upcast()
			}
			NativeKind::Switch => gtk::Switch::new().upcast(),
			NativeKind::Checkbox => gtk::CheckButton::new().upcast(),
			NativeKind::Radio => crate::radio::Radio::new().upcast(),
			NativeKind::Slider => {
				let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
				scale.set_draw_value(false);
				scale.upcast()
			}
			NativeKind::LinearProgressIndicator => gtk::ProgressBar::new().upcast(),
			NativeKind::ActivityIndicator => gtk::Spinner::new().upcast(),
			NativeKind::Image => {
				let picture = gtk::Picture::new();
				picture.set_can_shrink(true);
				picture.upcast()
			}
			NativeKind::ScrollView => {
				let scroll = gtk::ScrolledWindow::new();
				scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
				scroll.set_child(Some(&content));
				scroll.upcast()
			}
			NativeKind::Vector => crate::vector::Vector::new(crate::vector::role(props)).upcast(),
		};
		let css = gtk::CssProvider::new();
		let radio_anchor = if kind == NativeKind::Radio {
			let anchor = gtk::CheckButton::new();
			control
				.clone()
				.downcast::<gtk::CheckButton>()
				.expect("radio is a check button")
				.set_group(Some(&anchor));
			Some(anchor)
		} else {
			None
		};
		control
			.style_context()
			.add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
		// GTK fixes the accessible role of native controls itself; an
		// explicit role restyles plain containers at creation.
		if let Some(role) = props.aria().role {
			control.set_accessible_role(crate::accessibility::role(role));
		}
		if kind == NativeKind::Switch {
			let mut child = control.first_child();
			while let Some(widget) = child {
				if widget.css_name() == "slider" {
					widget
						.style_context()
						.add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
				}
				child = widget.next_sibling();
			}
		}
		frame.put(&control, graphene::Rect::new(0.0, 0.0, 0.0, 0.0), 0, true);
		let widget = Widget {
			kind,
			frame,
			control,
			content,
			props: props.clone(),
			parent: None,
			children: Vec::new(),
			rect: graphene::Rect::new(0.0, 0.0, 0.0, 0.0),
			css,
			css_text: RefCell::new(String::new()),
			animation: None,
			radio_anchor,
		};
		connect(&self.state, id, &widget);
		self.state.borrow_mut().widgets.insert(id, widget);
		self.refresh();
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		if let Some(widget) = self.state.borrow_mut().widgets.get_mut(&id) {
			widget.props = props.clone();

			if widget.kind == NativeKind::Vector {
				update_vector_role(widget);
			}
		}
		self.refresh();
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		let mut state = self.state.borrow_mut();
		let Some(widget) = state.widgets.get_mut(&id) else {
			return;
		};
		widget.rect = graphene::Rect::new(x, y, width, height);
		let parent = widget.parent;
		let frame = widget.frame.clone();
		let rect = widget.rect;

		if let Some(parent) = parent.and_then(|parent| state.widgets.get(&parent)) {
			parent.content.position(&frame, rect);
		} else if let Some(host) = frame.parent().and_downcast::<Frame>() {
			host.position(&frame, rect);
		}
		update_scroll_extents(&state);
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		let _guard = Suppression::new(&self.state);
		let mut state = self.state.borrow_mut();
		let Some(widget) = state.widgets.get_mut(&child) else {
			return;
		};
		widget.parent = Some(parent);
		let frame = widget.frame.clone();
		let rect = widget.rect;
		let Some(parent) = state.widgets.get_mut(&parent) else {
			return;
		};
		let index = index.min(parent.children.len());
		parent.children.insert(index, child);
		parent.content.put(&frame, rect, index, false);
		update_scroll_extents(&state);
		drop(state);
		self.refresh();
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		let guard = Suppression::new(&self.state);
		let mut state = self.state.borrow_mut();
		let Some(widget) = state.widgets.get_mut(&child) else {
			return;
		};
		widget.parent = None;
		let frame = widget.frame.clone();

		if let Some(parent) = state.widgets.get_mut(&parent) {
			parent.children.retain(|id| *id != child);
			parent.content.remove(&frame);
		}
		update_scroll_extents(&state);
		drop(state);
		self.refresh();
		drop(guard);
	}

	fn delete(&mut self, id: NodeId) {
		let guard = Suppression::new(&self.state);

		if let Some(mut widget) = self.state.borrow_mut().widgets.remove(&id) {
			stop(&mut widget);
			widget.frame.set_sensitive(false);
			widget.frame.set_visible(false);
		}
		drop(guard);
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		// Match the hinting GTK applies on the display that shows the tree,
		// or on the default display before any host exists.
		let host = self
			.state
			.borrow()
			.hosts
			.iter()
			.find_map(|host| host.frame.upgrade());
		Arc::new(host.map_or_else(
			|| GtkTextMeasurer::for_widget(&gtk::Label::new(None)),
			|frame| GtkTextMeasurer::for_widget(&frame),
		))
	}

	fn set_dark_mode(&mut self, dark: bool) {
		let _guard = Suppression::new(&self.state);
		self.state.borrow_mut().dark = Some(dark);
		// GTK's settings are display-wide by design; the host's explicit
		// appearance controls native widget chrome on this display.
		if let Some(settings) = gtk::Settings::default() {
			settings.set_gtk_application_prefer_dark_theme(dark);
		}
		self.refresh();
	}
}

fn sync_hosts(state: &mut State) {
	state.hosts.retain(|host| host.frame.upgrade().is_some());

	for host in &mut state.hosts {
		let Some(frame) = host.frame.upgrade() else {
			continue;
		};

		if host.root.is_some_and(|id| state.widgets.contains_key(&id)) {
			continue;
		}
		while let Some(child) = frame.first_child() {
			frame.remove(&child);
		}
		let root = state.widgets.iter().find_map(|(id, widget)| {
			(widget.parent.is_none() && widget.frame.parent().is_none()).then_some(*id)
		});
		host.root = root;

		if let Some(widget) = root.and_then(|id| state.widgets.get(&id)) {
			frame.put(&widget.frame, widget.rect, 0, false);
		}
	}
}

fn stop(widget: &mut Widget) {
	if let Some(source) = widget.animation.take() {
		source.remove();
	}

	if let Ok(spinner) = widget.control.clone().downcast::<gtk::Spinner>() {
		spinner.stop();
	}
}

/// Replaces a drawing's control when its label or explicit role now calls
/// for a different accessible role. GTK rejects role changes once assistive
/// technology has seen a widget; the control holds no state beyond its
/// drawing, which the next refresh restores.
fn update_vector_role(widget: &mut Widget) {
	let role = crate::vector::role(&widget.props);

	if widget.control.accessible_role() == role {
		return;
	}
	let control: gtk::Widget = crate::vector::Vector::new(role).upcast();
	control
		.style_context()
		.add_provider(&widget.css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
	widget.frame.remove(&widget.control);
	widget
		.frame
		.put(&control, graphene::Rect::new(0.0, 0.0, 0.0, 0.0), 0, true);
	widget.control = control;
}

fn update_scroll_extents(state: &State) {
	for widget in state
		.widgets
		.values()
		.filter(|widget| widget.kind == NativeKind::ScrollView)
	{
		let mut width = widget.rect.width();
		let mut height = widget.rect.height();

		for child in widget
			.children
			.iter()
			.filter_map(|id| state.widgets.get(id))
		{
			width = width.max(child.rect.x() + child.rect.width());
			height = height.max(child.rect.y() + child.rect.height());
		}
		widget
			.content
			.set_extent(width.ceil() as i32, height.ceil() as i32);
	}
}

fn configure(widget: &Widget, _dark: Option<bool>) {
	let props = &widget.props;
	let rtl = props.direction == Some(leptoskit_yoga::Direction::Rtl);
	widget.frame.style(props);
	widget.control.set_direction(if rtl {
		gtk::TextDirection::Rtl
	} else {
		gtk::TextDirection::Ltr
	});

	// A drawing may paint past its frame, as on every platform, unless the
	// element clips; the frame's own style already decides that.
	if !matches!(
		widget.kind,
		NativeKind::View | NativeKind::Custom(_) | NativeKind::Vector
	) {
		widget.frame.set_overflow(gtk::Overflow::Hidden);
	}
	let mut css = format!(
		"* {{ font-family: Sans; font-size: {}px; font-weight: {}; }}",
		props.text_style().font_size,
		match props.text_style().font_weight {
			leptoskit_native::FontWeight::Regular => 400,
			leptoskit_native::FontWeight::Medium => 500,
			leptoskit_native::FontWeight::Semibold => 600,
			leptoskit_native::FontWeight::Bold => 700,
		}
	);

	if let Some(color) = props.color {
		write!(
			css,
			"* {{ color: rgba({},{},{},{}); }}",
			color.red(),
			color.green(),
			color.blue(),
			f32::from(color.alpha()) / 255.0
		)
		.expect("formatting a String cannot fail");
	}

	if widget.kind == NativeKind::Pressable
		|| (widget.kind == NativeKind::Button && props.background.is_some())
	{
		css.push_str(
			"button { background: transparent; border: 0; box-shadow: none; padding: 0; }",
		);
	}
	if matches!(
		widget.kind,
		NativeKind::TextInput | NativeKind::PasswordInput
	) {
		// Yoga sizes an input from its text. The theme's minimum (34px in
		// Adwaita) would make the entry taller than that frame and
		// center its text and placeholder below the visible area.
		css.push_str("entry { min-height: 0; }");

		if props.is_custom_drawn() {
			css.push_str(
				"entry { background: transparent; border: 0; box-shadow: none; padding: 0; }",
			);
		}
	}
	if widget.kind == NativeKind::Switch {
		// The shared primitive defaults to 40×24. Adapt GTK's CSS size
		// request, retaining native theme paint and hit testing, so this
		// normal portable size does not clip a 50px theme minimum.
		css.push_str(
			"switch { min-width: 0; min-height: 0; } switch slider { min-width: 18px; min-height: \
			 18px; }",
		);
	}
	if *widget.css_text.borrow() != css {
		widget.css.load_from_data(&css);
		widget.css_text.replace(css);
	}

	if let Ok(label) = widget.control.clone().downcast::<gtk::Label>() {
		label.set_text(props.text.as_deref().unwrap_or(""));
		label.set_justify(match props.text_align.unwrap_or_default() {
			TextAlign::Start => gtk::Justification::Left,
			TextAlign::Center => gtk::Justification::Center,
			TextAlign::End => gtk::Justification::Right,
		});
		label.set_xalign(match props.text_align.unwrap_or_default() {
			TextAlign::Start => 0.0,
			TextAlign::Center => 0.5,
			TextAlign::End => 1.0,
		});
		let attrs = gtk::pango::AttrList::new();
		attrs.insert(gtk::pango::AttrFontDesc::new(&font(props.text_style())));
		label.set_attributes(Some(&attrs));
		label.layout().set_auto_dir(false);
	}

	if let Ok(button) = widget.control.clone().downcast::<gtk::Button>() {
		if widget.kind == NativeKind::Button {
			button.set_label(props.text.as_deref().unwrap_or(""));
			if let Some(label) = button.child().and_downcast::<gtk::Label>() {
				let alignment = props.text_align.unwrap_or(TextAlign::Center);
				label.set_xalign(match alignment {
					TextAlign::Start => 0.0,
					TextAlign::Center => 0.5,
					TextAlign::End => 1.0,
				});
				label.set_justify(match alignment {
					TextAlign::Start => gtk::Justification::Left,
					TextAlign::Center => gtk::Justification::Center,
					TextAlign::End => gtk::Justification::Right,
				});
				label.layout().set_auto_dir(false);
			}
		}
	}

	if let Ok(entry) = widget.control.clone().downcast::<gtk::Entry>() {
		let value = props
			.value
			.as_deref()
			.or(props.text.as_deref())
			.unwrap_or("");
		let physical = match props.text_align.unwrap_or_default() {
			TextAlign::Start => {
				if rtl {
					1.0
				} else {
					0.0
				}
			}
			TextAlign::Center => 0.5,
			TextAlign::End => {
				if rtl {
					0.0
				} else {
					1.0
				}
			}
		};
		// GtkText reverses xalign using its content's first strong direction,
		// which can differ from the declared Yoga direction. Compensate at
		// this native boundary so both Latin and Arabic honor logical start.
		let natural = if widget.kind == NativeKind::PasswordInput {
			gtk::pango::Direction::Neutral
		} else {
			gtk::pango::find_base_dir(value)
		};
		let resolved_rtl = match natural {
			gtk::pango::Direction::Rtl => true,
			gtk::pango::Direction::Ltr => false,
			_ if entry.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN) => {
				entry
					.display()
					.default_seat()
					.and_then(|seat| seat.keyboard())
					.is_some_and(|keyboard| keyboard.direction() == gtk::pango::Direction::Rtl)
			}
			_ => rtl,
		};
		EntryExt::set_alignment(
			&entry,
			if resolved_rtl {
				1.0 - physical
			} else {
				physical
			},
		);

		if entry.text() != value {
			let position = entry.position();
			entry.set_text(value);
			entry.set_position(position.min(value.chars().count() as i32));
		}
		entry.set_placeholder_text(props.placeholder.as_deref());
	}

	if let Ok(toggle) = widget.control.clone().downcast::<gtk::Switch>() {
		toggle.set_active(props.checked.unwrap_or(false));
		toggle.set_state(props.checked.unwrap_or(false));
	}

	if let Ok(toggle) = widget.control.clone().downcast::<gtk::CheckButton>() {
		toggle.set_active(props.checked.unwrap_or(false));

		if widget.kind == NativeKind::Radio {
			toggle.add_css_class("radio");
		}
	}

	if let Ok(scale) = widget.control.clone().downcast::<gtk::Scale>() {
		let slider = props.slider.unwrap_or_default();
		let increment = slider
			.step()
			.map_or(0.01, |step| step / (slider.max() - slider.min()));
		scale.set_increments(increment, (increment * 10.0).min(1.0));
		// A normalized GTK adjustment avoids overflow in native arithmetic for
		// otherwise valid finite shared ranges. Keyboard increments use f64
		// domain values in the capture controller below.
		scale.set_value((slider.value() - slider.min()) / (slider.max() - slider.min()));
		let value_text = format!(
			"{} (range {} to {})",
			slider.value(),
			slider.min(),
			slider.max()
		);
		scale.update_property(&[
			gtk::accessible::Property::ValueText(&value_text),
			gtk::accessible::Property::Description(&value_text),
		]);
	}

	if let (Ok(bar), Some(Progress::Determinate(value))) = (
		widget.control.clone().downcast::<gtk::ProgressBar>(),
		props.progress,
	) {
		bar.set_fraction(value.get());
	}

	if let Ok(picture) = widget.control.clone().downcast::<gtk::Picture>() {
		let file = props.source.as_deref().map(|source| {
			if source.starts_with("file://") {
				gtk::gio::File::for_uri(source)
			} else {
				gtk::gio::File::for_path(source)
			}
		});
		let unchanged = match (picture.file(), file.as_ref()) {
			(Some(old), Some(new)) => old.equal(new),
			(None, None) => true,
			_ => false,
		};

		if !unchanged {
			picture.set_file(file.as_ref());
		}
	}

	if let Ok(vector) = widget.control.clone().downcast::<crate::vector::Vector>() {
		vector.set_drawing(props.drawing.as_ref());
	}
	// Toolkit label/title setters can replace accessibility names. Apply the
	// explicit application label last, after every native content setter.
	widget
		.control
		.update_property(&[gtk::accessible::Property::Label(
			props
				.accessibility_label
				.as_deref()
				.unwrap_or(props.text.as_deref().unwrap_or("")),
		)]);
	// Newer GtkButton releases name themselves through a labelled-by
	// relation to their visible label, which outranks the label property.
	if props.accessibility_label.is_some() {
		widget
			.control
			.reset_relation(gtk::AccessibleRelation::LabelledBy);
	}
	crate::accessibility::apply(&widget.control, props, widget.kind);
}

fn interactive(state: &State, mut id: NodeId) -> bool {
	loop {
		let Some(widget) = state.widgets.get(&id) else {
			return false;
		};

		if widget.props.enabled == Some(false) || widget.props.hidden == Some(true) {
			return false;
		}

		let Some(parent) = widget.parent else {
			return true;
		};
		id = parent;
	}
}

fn emit(weak: &Weak<RefCell<State>>, flag: &Cell<bool>, id: NodeId, event: NativeEvent) {
	if flag.get() {
		return;
	}
	let Some(state) = weak.upgrade() else {
		return;
	};
	let listener = {
		let state = state.borrow();
		interactive(&state, id)
			.then(|| state.listener.clone())
			.flatten()
	};

	if let Some(listener) = listener {
		listener(id, event);
	}

	// Radio activation changes siblings before its signal arrives. Restore
	// the latest whole committed state after callbacks, including rejected
	// proposals and callbacks that replace or remove the original control.
	GtkBackend::refresh_state(&state);
}

fn connect(state: &Rc<RefCell<State>>, id: NodeId, widget: &Widget) {
	let weak = Rc::downgrade(state);
	let flag = state.borrow().suppressed.clone();
	let weak_map = weak.clone();
	let flag_map = flag.clone();
	widget.frame.connect_map(move |_| {
		if flag_map.get() {
			return;
		}
		if let Some(state) = weak_map.upgrade() {
			GtkBackend::refresh_state(&state);
		}
	});
	let weak_map = weak.clone();
	let flag_map = flag.clone();
	widget.frame.connect_unmap(move |_| {
		if flag_map.get() {
			return;
		}
		if let Some(state) = weak_map.upgrade() {
			GtkBackend::refresh_state(&state);
		}
	});

	if let Ok(button) = widget.control.clone().downcast::<gtk::Button>() {
		let weak = weak.clone();
		let flag = flag.clone();
		button.connect_clicked(move |_| emit(&weak, &flag, id, NativeEvent::Press));
	}

	if let Ok(entry) = widget.control.clone().downcast::<gtk::Entry>() {
		let weak = weak.clone();
		let flag = flag.clone();
		entry.connect_changed(move |entry| {
			emit(
				&weak,
				&flag,
				id,
				NativeEvent::Change {
					value: entry.text().to_string(),
				},
			);
		});
		let weak = Rc::downgrade(state);
		let flag = state.borrow().suppressed.clone();
		entry.connect_activate(move |entry| {
			emit(
				&weak,
				&flag,
				id,
				NativeEvent::Submit {
					value: entry.text().to_string(),
				},
			);
		});
	}

	if let Ok(toggle) = widget.control.clone().downcast::<gtk::Switch>() {
		let weak = weak.clone();
		let flag = flag.clone();
		toggle.connect_state_set(move |_, value| {
			emit(&weak, &flag, id, NativeEvent::Toggle { checked: value });
			glib::Propagation::Stop
		});
	}

	if let Ok(toggle) = widget.control.clone().downcast::<gtk::CheckButton>() {
		let weak = weak.clone();
		let flag = flag.clone();
		let radio = widget.kind == NativeKind::Radio;
		toggle.connect_toggled(move |toggle| {
			if !radio || toggle.is_active() {
				emit(
					&weak,
					&flag,
					id,
					NativeEvent::Toggle {
						checked: toggle.is_active(),
					},
				);
			}
		});
	}

	if let Ok(scale) = widget.control.clone().downcast::<gtk::Scale>() {
		let weak = weak.clone();
		let flag = flag.clone();
		scale.connect_value_changed(move |scale| {
			if flag.get() {
				return;
			}
			let slider = weak.upgrade().and_then(|state| {
				state
					.borrow()
					.widgets
					.get(&id)
					.map(|widget| widget.props.slider.unwrap_or_default())
			});

			if let Some(slider) = slider {
				if let Some(value) = slider.proposed_value(
					scale
						.value()
						.mul_add(slider.max() - slider.min(), slider.min()),
				) {
					emit(&weak, &flag, id, NativeEvent::ValueChange { value });
				}
			}
		});
	}

	if let Ok(scroll) = widget.control.clone().downcast::<gtk::ScrolledWindow>() {
		for adjustment in [scroll.hadjustment(), scroll.vadjustment()] {
			let weak = weak.clone();
			let flag = flag.clone();
			let scroll = scroll.downgrade();
			adjustment.connect_value_changed(move |_| {
				if let Some(scroll) = scroll.upgrade() {
					emit(
						&weak,
						&flag,
						id,
						NativeEvent::Scroll {
							offset_x: scroll.hadjustment().value() as f32,
							offset_y: scroll.vadjustment().value() as f32,
						},
					);
				}
			});
		}
	}

	if matches!(
		widget.kind,
		NativeKind::Button
			| NativeKind::Pressable
			| NativeKind::TextInput
			| NativeKind::PasswordInput
			| NativeKind::Switch
			| NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
	) {
		let focus = gtk::EventControllerFocus::new();
		let weak_focus = weak.clone();
		let flag_focus = flag.clone();
		focus.connect_enter(move |_| emit(&weak_focus, &flag_focus, id, NativeEvent::Focus));
		let weak_focus = weak.clone();
		let flag_focus = flag.clone();
		focus.connect_leave(move |_| emit(&weak_focus, &flag_focus, id, NativeEvent::Blur));
		widget.control.add_controller(focus);
	}

	if matches!(widget.kind, NativeKind::Radio | NativeKind::Slider) {
		let key = gtk::EventControllerKey::new();
		key.set_propagation_phase(gtk::PropagationPhase::Capture);
		key.connect_key_pressed(move |_, key, _, _| {
			let Some(state) = weak.upgrade() else {
				return glib::Propagation::Proceed;
			};
			let proposal = key_proposal(&state.borrow(), id, key);

			if let Some((target, event, control)) = proposal {
				control.grab_focus();
				emit(&weak, &flag, target, event);
				return glib::Propagation::Stop;
			}
			glib::Propagation::Proceed
		});
		widget.control.add_controller(key);
	}
}

fn key_proposal(
	state: &State,
	id: NodeId,
	key: gdk::Key,
) -> Option<(NodeId, NativeEvent, gtk::Widget)> {
	let widget = state.widgets.get(&id)?;

	if !interactive(state, id) {
		return None;
	}
	let rtl = widget.props.direction == Some(leptoskit_yoga::Direction::Rtl);
	let forward =
		key == gdk::Key::Down || key == if rtl { gdk::Key::Left } else { gdk::Key::Right };
	let backward = key == gdk::Key::Up || key == if rtl { gdk::Key::Right } else { gdk::Key::Left };

	if widget.kind == NativeKind::Slider {
		let slider = widget.props.slider.unwrap_or_default();
		let increment = slider
			.step()
			.unwrap_or((slider.max() - slider.min()) / 100.0);
		let value = match key {
			gdk::Key::Home => slider.min(),
			gdk::Key::End => slider.max(),
			gdk::Key::Up => slider.value() + increment,
			gdk::Key::Left if rtl => slider.value() + increment,
			gdk::Key::Right if !rtl => slider.value() + increment,
			gdk::Key::Down | gdk::Key::Left | gdk::Key::Right => slider.value() - increment,
			_ => return None,
		};
		return Some((
			id,
			NativeEvent::ValueChange {
				value: slider.proposed_value(value)?,
			},
			widget.control.clone(),
		));
	}

	if widget.kind != NativeKind::Radio || (!forward && !backward) {
		return None;
	}
	let group = widget.props.radio_group.as_ref()?;
	let mut root = id;

	while let Some(parent) = state.widgets.get(&root).and_then(|widget| widget.parent) {
		root = parent;
	}
	let mut members = Vec::new();
	collect_radios(state, root, group, &mut members);
	let current = members.iter().position(|member| *member == id)?;
	let next = if forward {
		(current + 1) % members.len()
	} else {
		(current + members.len() - 1) % members.len()
	};
	let target = members[next];
	Some((
		target,
		NativeEvent::Toggle { checked: true },
		state.widgets.get(&target)?.control.clone(),
	))
}

fn collect_radios(
	state: &State,
	id: NodeId,
	group: &leptoskit_native::RadioGroup,
	members: &mut Vec<NodeId>,
) {
	let Some(widget) = state.widgets.get(&id) else {
		return;
	};

	if widget.kind == NativeKind::Radio
		&& widget.props.radio_group.as_ref() == Some(group)
		&& interactive(state, id)
	{
		members.push(id);
	}

	for child in &widget.children {
		collect_radios(state, *child, group, members);
	}
}

fn collect_visible_group(state: &State, id: NodeId, group: &str, members: &mut Vec<NodeId>) {
	let Some(widget) = state.widgets.get(&id) else {
		return;
	};

	if widget.props.hidden == Some(true) {
		return;
	}

	if widget.kind == NativeKind::Radio
		&& widget
			.props
			.radio_group
			.as_ref()
			.is_some_and(|name| name.name() == group)
	{
		members.push(id);
	}

	for child in &widget.children {
		collect_visible_group(state, *child, group, members);
	}
}
