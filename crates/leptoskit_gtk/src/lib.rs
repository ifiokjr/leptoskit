//! GTK4 widgets consuming the shared Yoga mutation stream.
//!
//! GTK objects stay on their creating thread. The backend is consequently
//! neither `Send` nor `Sync`; its stateless Pango measurer can cross threads.
//! Every node has a zero-minimum outer frame, so native control minimum sizes
//! never override the application's Yoga geometry.

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod accessibility;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod backend;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod frame;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod measure;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod radio;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod vector;

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub use backend::EventListener;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub use backend::GtkBackend;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub use gtk;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub use measure::GtkTextMeasurer;

/// Runs GTK's event loop while a desktop window is visible.
///
/// Applications embedding GTK in an existing event loop may drive the `GLib`
/// main context themselves instead. Initialize GTK before calling this
/// function.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
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
