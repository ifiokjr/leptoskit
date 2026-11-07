//! Errors from validating or presenting watch surfaces.

use thiserror::Error;

/// Something a watch surface rejected before or during presentation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WatchError {
	/// The content tree is deeper than the surface's budget allows.
	#[error("watch content is {depth} levels deep; budget allows {max}")]
	TooDeep {
		/// Measured depth.
		depth: u8,
		/// Budgeted maximum depth.
		max: u8,
	},
	/// The content tree has more nodes than the surface's budget allows.
	#[error("watch content has {nodes} nodes; budget allows {max}")]
	TooManyNodes {
		/// Measured node count.
		nodes: u16,
		/// Budgeted maximum nodes.
		max: u16,
	},
	/// Visible text is longer than the surface's budget allows.
	#[error("watch content has {chars} text characters; budget allows {max}")]
	TooMuchText {
		/// Measured character count.
		chars: u16,
		/// Budgeted maximum characters.
		max: u16,
	},
	/// A widget kind that watch surfaces refuse (for example a password field).
	#[error("watch surfaces do not accept `{0}` widgets")]
	UnsupportedKind(&'static str),
	/// A page or ambient id is empty or duplicated.
	#[error("watch surface id `{0}` is invalid")]
	InvalidId(String),
	/// Two surfaces share the same id inside one app.
	#[error("watch surface id `{0}` is used more than once")]
	DuplicateId(String),
	/// A stack or pager needs at least one page.
	#[error("a watch app with {0:?} navigation needs at least one page")]
	MissingPage(crate::WatchNavigation),
	/// The host cannot present this surface yet (stub / experimental).
	#[error("{0}")]
	Unavailable(String),
	/// An unknown watch platform name.
	#[error("unknown watch platform `{0}` (expected wearos or watchos)")]
	UnknownKind(String),
}
