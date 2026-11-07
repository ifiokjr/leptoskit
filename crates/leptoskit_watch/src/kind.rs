//! Which watch operating system a surface targets.

/// The watch operating system a [`crate::WatchApp`] is built for.
///
/// Wear OS and watchOS share the same surface vocabulary (pages, tiles,
/// glances, complications) but ship through different hosts, manifests, and
/// store pipelines. Name them explicitly so apps never assume one phone tree
/// covers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchKind {
	/// Android Wear OS (tiles, complications, watch apps).
	WearOs,
	/// Apple watchOS (`WidgetKit` glances, `ClockKit` complications, watch
	/// apps).
	WatchOs,
}

impl WatchKind {
	/// Stable configuration / CLI name: `wearos` or `watchos`.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::WearOs => "wearos",
			Self::WatchOs => "watchos",
		}
	}

	/// Human-readable product name.
	#[must_use]
	pub const fn display_name(self) -> &'static str {
		match self {
			Self::WearOs => "Wear OS",
			Self::WatchOs => "watchOS",
		}
	}
}

impl std::fmt::Display for WatchKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

impl std::str::FromStr for WatchKind {
	type Err = crate::WatchError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"wearos" | "wear-os" | "wear_os" => Ok(Self::WearOs),
			"watchos" | "watch-os" | "watch_os" => Ok(Self::WatchOs),
			other => Err(crate::WatchError::UnknownKind(other.to_owned())),
		}
	}
}
