//! Generated native registration for the scoped process service.

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod desktop;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::ShellPlugin;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::from_config;

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod unavailable {
	use std::future::Future;
	use std::pin::Pin;

	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::MethodId;
	use leptoskit_bridge::async_host::AsyncPlugin;
	use leptoskit_bridge::async_host::CancellationToken;
	use leptoskit_bridge::async_host::PluginFuture;

	/// Native mobile platforms do not expose a process service.
	pub struct ShellPlugin;

	/// Rejects installation on unsupported platforms without parsing or running
	/// any command.
	pub fn from_config(_: &str) -> Result<ShellPlugin, Error> {
		Err(Error::Unavailable)
	}

	impl AsyncPlugin for ShellPlugin {
		fn name(&self) -> &'static str {
			"shell"
		}

		fn methods(&self) -> &'static [MethodId] {
			&[crate::EXECUTE]
		}

		fn reply_limit(&self, _: MethodId) -> Result<usize, Error> {
			Err(Error::Unavailable)
		}

		fn invoke(&self, _: MethodId, _: Bytes, _: CancellationToken) -> PluginFuture<'_> {
			Box::pin(async { Err(Error::Unavailable) })
		}

		fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
			Box::pin(async {})
		}
	}
}
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub use unavailable::ShellPlugin;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub use unavailable::from_config;
