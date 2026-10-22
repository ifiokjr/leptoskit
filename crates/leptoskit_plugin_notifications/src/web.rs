//! Non-persistent browser notifications. Request permission from a user gesture
//! in a secure context. Browsers requiring service-worker notifications report
//! Unavailable; this module does not install a persistent worker.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use super::Error;
use super::Notification;
use super::Permission;

fn supported() -> Result<(), Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	if js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("Notification"))
		.map_err(|_| Error::Native)?
		.is_function()
	{
		Ok(())
	} else {
		Err(Error::Unavailable)
	}
}
fn state(value: web_sys::NotificationPermission) -> Result<Permission, Error> {
	match value {
		web_sys::NotificationPermission::Default => Ok(Permission::NotDetermined),
		web_sys::NotificationPermission::Denied => Ok(Permission::Denied),
		web_sys::NotificationPermission::Granted => Ok(Permission::Granted),
		_ => Err(Error::Native),
	}
}
fn map_error(error: &JsValue) -> Error {
	if error.is_instance_of::<js_sys::TypeError>() {
		return Error::Unavailable;
	}
	if let Some(exception) = error.dyn_ref::<web_sys::DomException>() {
		match exception.name().as_str() {
			"NotAllowedError" | "SecurityError" => return Error::PermissionDenied,
			"NotSupportedError" => return Error::Unavailable,
			_ => {}
		}
	}
	Error::Native
}
/// Reads browser permission without opening a prompt.
pub fn permission() -> Result<Permission, Error> {
	supported()?;
	state(web_sys::Notification::permission())
}
/// Requests permission explicitly; cancellation cannot dismiss the OS prompt.
pub async fn request_permission() -> Result<Permission, Error> {
	supported()?;
	let promise = web_sys::Notification::request_permission().map_err(|error| map_error(&error))?;
	JsFuture::from(promise)
		.await
		.map_err(|error| map_error(&error))?;
	permission()
}
/// Requests browser delivery without opening a permission dialog implicitly.
/// Retain the returned handle if later cancellation is required.
pub fn show(notification: &Notification) -> Result<Handle, Error> {
	if permission()? != Permission::Granted {
		return Err(Error::PermissionDenied);
	}
	let options = web_sys::NotificationOptions::new();
	options.set_body(notification.body());
	options.set_tag(&format!(
		"leptoskit.notifications.{}",
		notification.id().as_str()
	));
	options.set_silent(Some(true));
	let value = web_sys::Notification::new_with_options(notification.title(), &options)
		.map_err(|error| map_error(&error))?;
	Ok(Handle(value))
}
/// Owns the browser object, not the notification's delivery lifetime. Dropping
/// this handle does not retract a notification the user may still need to read.
pub struct Handle(web_sys::Notification);
impl Handle {
	/// Dismisses this notification. Safe when the user has already dismissed
	/// it.
	pub fn cancel(&self) {
		self.0.close();
	}
}
