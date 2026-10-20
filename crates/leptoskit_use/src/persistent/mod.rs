//! Optional persistence adapters selected by the application host.

/// Errors returned by the built-in persistent storage adapters.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
	/// The environment does not expose the requested storage capability.
	#[error("storage is unavailable: {0}")]
	Unavailable(String),
	/// Native filesystem access failed.
	#[error("storage I/O failed: {0}")]
	Io(#[from] std::io::Error),
	/// A value could not be encoded or decoded as JSON.
	#[error("storage encoding failed: {0}")]
	Codec(#[from] serde_json::Error),
	/// Browser access failed, for example because of quota or privacy settings.
	#[error("browser storage failed: {0}")]
	Browser(String),
}

#[cfg(not(target_arch = "wasm32"))]
mod file;
#[cfg(not(target_arch = "wasm32"))]
pub use file::FileStorage;
#[cfg(not(target_arch = "wasm32"))]
pub use file::FileStorage as LocalStorage;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::WebStorage;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::WebStorage as LocalStorage;
