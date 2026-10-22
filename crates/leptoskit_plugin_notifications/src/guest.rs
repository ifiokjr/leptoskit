//! Cached native notification client.
use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct Notifications(leptoskit_bridge::guest::Client);
impl Notifications {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Queries settings without prompting.
	pub async fn permission(self) -> Result<Permission, Error> {
		Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
	}

	/// Requests OS permission. The OS dialog itself may outlive cancellation.
	pub async fn request_permission(self) -> Result<Permission, Error> {
		Permission::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
	}

	/// Submits immediate local content. Acceptance is not proof of visible
	/// delivery.
	pub async fn show(self, notification: &Notification) -> Result<(), Error> {
		self.empty(SHOW, &notification.encode()?).await
	}

	/// Removes the named notification, including pending native work.
	pub async fn cancel(self, id: &Identifier) -> Result<(), Error> {
		self.empty(CANCEL, &id.encode()?).await
	}

	/// Removes this plugin's notifications without touching other app
	/// providers.
	pub async fn cancel_all(self) -> Result<(), Error> {
		self.empty(CANCEL_ALL, &[]).await
	}

	/// Creates/updates Android channel metadata; does not override user
	/// settings.
	pub async fn configure_channel(self, channel: &Channel) -> Result<(), Error> {
		self.empty(CONFIGURE_CHANNEL, &channel.encode()?).await
	}

	/// Deletes an Android channel. Other platforms report Unavailable.
	pub async fn delete_channel(self, id: &Identifier) -> Result<(), Error> {
		self.empty(DELETE_CHANNEL, &id.encode()?).await
	}

	async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
		if self.0.call(method, payload)?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
