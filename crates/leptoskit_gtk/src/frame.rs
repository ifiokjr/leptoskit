//! Exact Yoga geometry with native child controls and retained GTK snapshots.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use leptoskit_native::Color;
use leptoskit_native::NativeProps;

type ResizeListener = Rc<dyn Fn(i32, i32)>;

#[derive(Clone)]
pub(crate) struct Child {
	pub widget: gtk::Widget,
	pub rect: graphene::Rect,
	pub native: bool,
}

mod imp {
	use super::*;

	#[derive(Default)]
	pub struct Frame {
		pub(super) children: RefCell<Vec<Child>>,
		pub(super) props: RefCell<NativeProps>,
		pub(super) extent: Cell<Option<(i32, i32)>>,
		pub(super) css: gtk::CssProvider,
		pub(super) resize: RefCell<Option<ResizeListener>>,
		pub(super) last_size: Cell<(i32, i32)>,
	}

	#[glib::object_subclass]
	impl ObjectSubclass for Frame {
		type ParentType = gtk::Widget;
		type Type = super::Frame;

		const NAME: &'static str = "LeptoskitYogaFrame";
	}

	impl ObjectImpl for Frame {
		fn dispose(&self) {
			for child in self.children.take() {
				child.widget.unparent();
			}
		}
	}

	impl WidgetImpl for Frame {
		fn contains(&self, x: f64, y: f64) -> bool {
			let obj = self.obj();
			let props = self.props.borrow();
			let bounds = graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32);
			gsk::RoundedRect::from_rect(bounds, props.corner_radius.unwrap_or(0.0))
				.contains_point(graphene::Point::new(x as f32, y as f32))
		}

		fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
			let size = self.extent.get().map_or(0, |(width, height)| {
				match orientation {
					gtk::Orientation::Horizontal => width,
					_ => height,
				}
			});
			(size, size, -1, -1)
		}

		fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
			for child in self.children.borrow().iter() {
				let rect = if child.native {
					graphene::Rect::new(0.0, 0.0, width as f32, height as f32)
				} else {
					child.rect
				};
				let target_width = rect.width().round().max(0.0) as i32;
				let target_height = rect.height().round().max(0.0) as i32;
				// Measure the width without a height constraint. GTK 4.14
				// answers a width-for-height query for a wrapping label with
				// a wider-than-allocated width, which then unwraps its text.
				let minimum_width = child.widget.measure(gtk::Orientation::Horizontal, -1).0;
				let allocated_width = minimum_width.max(target_width);
				let minimum_height = child
					.widget
					.measure(gtk::Orientation::Vertical, allocated_width)
					.0;
				let allocated_height = minimum_height.max(target_height);
				// GTK clamps native interiors to their toolkit minimum. The
				// zero-minimum outer frame clips tiny controls, preserving font
				// and chrome sizes instead of silently shrinking their
				// contents. Native controls center their text vertically, so
				// an oversized one is centered too: the clip then removes
				// chrome rather than the text Yoga measured.
				let overflow = if child.native {
					(allocated_height - target_height) / 2
				} else {
					0
				};
				let transform = gsk::Transform::new()
					.translate(&graphene::Point::new(rect.x(), rect.y() - overflow as f32));
				child
					.widget
					.allocate(allocated_width, allocated_height, -1, Some(transform));
			}
			if self.last_size.replace((width, height)) != (width, height) {
				if let Some(resize) = self.resize.borrow().clone() {
					let weak = self.obj().downgrade();
					glib::idle_add_local_once(move || {
						if let Some(frame) = weak.upgrade() {
							if frame.is_mapped() && frame.imp().last_size.get() == (width, height) {
								resize(width, height);
							}
						}
					});
				}
			}
		}

		fn snapshot(&self, snapshot: &gtk::Snapshot) {
			let obj = self.obj();
			let props = self.props.borrow();
			let bounds = graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32);
			let rounded = gsk::RoundedRect::from_rect(bounds, props.corner_radius.unwrap_or(0.0));
			let clip = props.clip.unwrap_or(false) || props.corner_radius.unwrap_or(0.0) > 0.0;

			if clip {
				snapshot.push_rounded_clip(&rounded);
			}

			if let Some(color) = props.background {
				snapshot.append_color(&rgba(color), &bounds);
			}

			for child in self.children.borrow().iter() {
				if child.widget.is_visible() {
					// Native controls may be allocated beyond the frame; clip
					// their chrome. A drawing paints past its frame unless the
					// frame itself clips, as on every platform.
					let clip_child = child.native
						&& !child.widget.is::<super::Frame>()
						&& !child.widget.is::<crate::vector::Vector>();
					if clip_child {
						snapshot.push_clip(&bounds);
					}
					obj.snapshot_child(&child.widget, snapshot);
					if clip_child {
						snapshot.pop();
					}
				}
			}

			if let Some(color) = props.border_color {
				let width = props.border_width.unwrap_or(0.0);
				snapshot.append_border(&rounded, &[width; 4], &[rgba(color); 4]);
			}

			if clip {
				snapshot.pop();
			}
		}
	}
}

glib::wrapper! {
	pub struct Frame(ObjectSubclass<imp::Frame>)
		@extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Frame {
	pub fn set_resize_listener(&self, listener: impl Fn(i32, i32) + 'static) {
		self.imp().resize.replace(Some(Rc::new(listener)));
	}

	pub fn new() -> Self {
		Self::styled(glib::Object::new())
	}

	/// A frame whose subtree is presented only through its parent's
	/// accessible label, such as a Pressable's content. GTK never presents
	/// this role or its descendants, and unlike the hidden state, which newer
	/// GTK releases reset whenever a widget maps, the role is permanent.
	pub fn decorative() -> Self {
		Self::styled(
			glib::Object::builder()
				.property("accessible-role", gtk::AccessibleRole::None)
				.build(),
		)
	}

	fn styled(frame: Self) -> Self {
		frame
			.style_context()
			.add_provider(&frame.imp().css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
		frame
	}

	pub fn put(
		&self,
		widget: &impl IsA<gtk::Widget>,
		rect: graphene::Rect,
		index: usize,
		native: bool,
	) {
		let widget = widget.as_ref();
		let next = self
			.imp()
			.children
			.borrow()
			.get(index)
			.map(|child| child.widget.clone());
		widget.insert_before(self, next.as_ref());
		let mut children = self.imp().children.borrow_mut();
		let index = index.min(children.len());
		children.insert(
			index,
			Child {
				widget: widget.clone(),
				rect,
				native,
			},
		);
		self.queue_allocate();
	}

	pub fn remove(&self, widget: &impl IsA<gtk::Widget>) {
		let widget = widget.as_ref();
		self.imp()
			.children
			.borrow_mut()
			.retain(|child| child.widget != *widget);

		if widget.parent().as_ref() == Some(self.upcast_ref()) {
			widget.unparent();
		}
		self.queue_allocate();
	}

	pub fn position(&self, widget: &impl IsA<gtk::Widget>, rect: graphene::Rect) {
		if let Some(child) = self
			.imp()
			.children
			.borrow_mut()
			.iter_mut()
			.find(|child| child.widget == *widget.as_ref())
		{
			child.rect = rect;
		}
		self.queue_allocate();
	}

	pub fn style(&self, props: &NativeProps) {
		// GTK picking clips against its CSS padding box, whereas GSK clips
		// snapshots. Set the same radius in both paths so rounded corners
		// reject pointer input as well as invisible pixels.
		if self.imp().props.borrow().corner_radius != props.corner_radius {
			self.imp().css.load_from_data(&format!(
				"* {{ border-radius: {}px; }}",
				props.corner_radius.unwrap_or(0.0)
			));
		}
		self.imp().props.replace(props.clone());
		self.set_opacity(f64::from(props.opacity.unwrap_or(1.0)));
		self.set_visible(!props.hidden.unwrap_or(false));
		self.set_sensitive(props.enabled.unwrap_or(true));
		self.set_overflow(
			if props.clip.unwrap_or(false) || props.corner_radius.unwrap_or(0.0) > 0.0 {
				gtk::Overflow::Hidden
			} else {
				gtk::Overflow::Visible
			},
		);
		self.queue_draw();
	}

	pub fn set_extent(&self, width: i32, height: i32) {
		self.imp().extent.set(Some((width, height)));
		self.queue_resize();
	}
}

impl Default for Frame {
	fn default() -> Self {
		Self::new()
	}
}

fn rgba(color: Color) -> gtk::gdk::RGBA {
	gtk::gdk::RGBA::new(
		f32::from(color.red()) / 255.0,
		f32::from(color.green()) / 255.0,
		f32::from(color.blue()) / 255.0,
		f32::from(color.alpha()) / 255.0,
	)
}
