//! Cached native home-widgets client.
use super::*;

/// Resolves the host once, then uses numeric method identifiers.
#[derive(Debug, Clone, Copy)]
pub struct HomeWidgets(leptoskit_bridge::guest::Client);

impl HomeWidgets {
	/// Resolves an explicitly installed host plugin.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(Self)
	}

	/// Reports what this host can do without touching widget storage.
	pub async fn capability(self) -> Result<Capability, Error> {
		Capability::decode(&self.0.call(CAPABILITY, &[])?.await?)
	}

	/// Publishes a snapshot for the widget extension to read. Acceptance is
	/// not proof that a home-screen instance exists or refreshed.
	pub async fn set(self, snapshot: &Snapshot) -> Result<(), Error> {
		self.empty(SET, &snapshot.encode()?).await
	}

	/// Reads the snapshot previously published for `kind`.
	pub async fn get(self, kind: &Kind) -> Result<Snapshot, Error> {
		Snapshot::decode(&self.0.call(GET, &kind.encode()?)?.await?)
	}

	/// Removes the published snapshot for `kind`.
	pub async fn clear(self, kind: &Kind) -> Result<(), Error> {
		self.empty(CLEAR, &kind.encode()?).await
	}

	/// Asks the OS to reload widget timelines. `None` reloads every kind this
	/// plugin owns. Reload does not create or pin a widget.
	pub async fn reload(self, kind: Option<&Kind>) -> Result<(), Error> {
		self.empty(RELOAD, &encode_reload(kind)?).await
	}

	/// Reports whether any instance of `kind` is pinned, when the platform can
	/// tell. WidgetKit returns [`Installed::Unknown`].
	pub async fn installed(self, kind: &Kind) -> Result<Installed, Error> {
		Installed::decode(&self.0.call(INSTALLED, &kind.encode()?)?.await?)
	}

	async fn empty(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
		if self.0.call(method, payload)?.await?.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
