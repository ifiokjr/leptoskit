//! Native radio chrome with an explicit radio accessibility role.

use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
	use super::*;

	#[derive(Default)]
	pub struct Radio;

	#[glib::object_subclass]
	impl ObjectSubclass for Radio {
		type ParentType = gtk::CheckButton;
		type Type = super::Radio;

		const NAME: &'static str = "LeptoskitRadio";

		fn class_init(class: &mut Self::Class) {
			class.set_accessible_role(gtk::AccessibleRole::Radio);
		}
	}

	impl ObjectImpl for Radio {}
	impl WidgetImpl for Radio {}
	impl CheckButtonImpl for Radio {}
}

glib::wrapper! {
	pub struct Radio(ObjectSubclass<imp::Radio>)
		@extends gtk::Widget, gtk::CheckButton,
		@implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Actionable;
}

impl Radio {
	pub fn new() -> Self {
		glib::Object::new()
	}
}

impl Default for Radio {
	fn default() -> Self {
		Self::new()
	}
}
