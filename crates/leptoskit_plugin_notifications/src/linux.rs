//! Linux desktop notification service over D-Bus. Calls can block up to five
//! seconds; use away from the rendering thread. No desktop notification is
//! emitted by constructing the client or querying its capabilities.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use dbus::arg::RefArg;
use dbus::arg::Variant;
use dbus::blocking::Connection;

use super::Error;
use super::Notification;

/// Connection and app identity used for immediate desktop notifications.
pub struct Notifications {
	connection: Rc<Connection>,
	application: String,
}
impl Notifications {
	/// Connects to the session bus. The app name is display metadata, not a
	/// security identity or a substitute for desktop packaging.
	pub fn new(application: impl Into<String>) -> Result<Self, Error> {
		let application = application.into();
		super::validate_text(&application, 256, false, true)?;
		let connection = Connection::new_session().map_err(|_| Error::Unavailable)?;
		Ok(Self {
			connection: Rc::new(connection),
			application,
		})
	}

	/// Requests immediate delivery. Linux generates its own ID; retain the
	/// returned handle for cancellation. The portable ID is not used to replace
	/// another process's notification. Android channel settings do not apply.
	pub fn show(&self, notification: &Notification) -> Result<Handle, Error> {
		let proxy = self.connection.with_proxy(
			"org.freedesktop.Notifications",
			"/org/freedesktop/Notifications",
			Duration::from_secs(5),
		);
		// Escape body markup so notification text cannot inject
		// hyperlinks/images.
		let body = notification
			.body()
			.replace('&', "&amp;")
			.replace('<', "&lt;")
			.replace('>', "&gt;");
		let hints: HashMap<&str, Variant<Box<dyn RefArg>>> =
			HashMap::from([("suppress-sound", Variant(Box::new(true) as Box<dyn RefArg>))]);
		let (id,): (u32,) = proxy
			.method_call(
				"org.freedesktop.Notifications",
				"Notify",
				(
					self.application.as_str(),
					0_u32,
					"",
					notification.title(),
					body.as_str(),
					Vec::<String>::new(),
					hints,
					-1_i32,
				),
			)
			.map_err(|_| Error::Native)?;

		if id == 0 {
			return Err(Error::Native);
		}

		Ok(Handle {
			connection: Rc::clone(&self.connection),
			id,
		})
	}
}

/// Cancellation handle. Dropping it does not retract a delivered notification.
pub struct Handle {
	connection: Rc<Connection>,
	id: u32,
}
impl Handle {
	/// Asks the desktop service to dismiss this notification. A service may
	/// return an error if the user already dismissed it; that error is
	/// preserved.
	pub fn cancel(&self) -> Result<(), Error> {
		let proxy = self.connection.with_proxy(
			"org.freedesktop.Notifications",
			"/org/freedesktop/Notifications",
			Duration::from_secs(5),
		);
		proxy
			.method_call::<(), _, _, _>(
				"org.freedesktop.Notifications",
				"CloseNotification",
				(self.id,),
			)
			.map_err(|_| Error::Native)
	}
}
