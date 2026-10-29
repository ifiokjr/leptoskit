//! GTK4 widgets consuming the shared Yoga mutation stream.
//!
//! GTK objects stay on their creating thread. The backend is consequently
//! neither `Send` nor `Sync`; its stateless Pango measurer can cross threads.
//! Every node has a zero-minimum outer frame, so native control minimum sizes
//! never override the application's Yoga geometry.

#[cfg(target_os = "linux")]
mod accessibility;
#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
mod frame;
#[cfg(target_os = "linux")]
mod measure;
#[cfg(target_os = "linux")]
mod radio;
#[cfg(target_os = "linux")]
mod vector;

#[cfg(target_os = "linux")]
pub use backend::EventListener;
#[cfg(target_os = "linux")]
pub use backend::GtkBackend;
#[cfg(target_os = "linux")]
pub use gtk;
#[cfg(target_os = "linux")]
pub use measure::GtkTextMeasurer;

/// Runs GTK's event loop while a desktop window is visible.
///
/// Applications embedding GTK in an existing event loop may drive the `GLib`
/// main context themselves instead. Initialize GTK before calling this
/// function.
#[cfg(target_os = "linux")]
pub fn run_application() {
	use gtk::prelude::*;
	let context = gtk::glib::MainContext::default();

	while gtk::Window::list_toplevels()
		.iter()
		.any(WidgetExt::is_visible)
	{
		context.iteration(true);
	}
}
