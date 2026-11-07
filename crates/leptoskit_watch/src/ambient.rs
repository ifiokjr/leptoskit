//! Ambient watch surfaces: tiles, glances, and complications.

use leptoskit_native::Element;

use crate::WatchBudget;
use crate::WatchError;
use crate::budget;
use crate::page::validate_id;

/// How often an ambient surface may ask the host to refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RefreshPolicy {
	/// The host refreshes only when the app asks.
	#[default]
	Manual,
	/// About once a minute when the system allows it.
	Minute,
	/// About once every fifteen minutes.
	QuarterHour,
	/// About once an hour.
	Hour,
}

/// Complication slot families shared across Wear OS and watchOS.
///
/// Platform hosts map these onto `ClockKit` families or Wear complication
/// types. Not every family exists on every face; unsupported families fail at
/// present time with [`WatchError::Unavailable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComplicationFamily {
	/// Circular / round slot.
	Circular,
	/// Smaller circular slot.
	CircularSmall,
	/// Wide rectangular slot.
	Rectangular,
	/// Inline text along the bezel or in a list.
	Inline,
	/// Corner slot (watchOS).
	Corner,
	/// Extra-large slot that dominates the face.
	ExtraLarge,
}

/// A Wear OS tile or a watchOS accessory widget: ambient, tappable,
/// short-lived.
#[derive(Clone)]
pub struct WatchTile {
	id: String,
	content: Element,
	refresh: RefreshPolicy,
	budget: WatchBudget,
}

impl std::fmt::Debug for WatchTile {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WatchTile")
			.field("id", &self.id)
			.field("refresh", &self.refresh)
			.field("budget", &self.budget)
			.finish_non_exhaustive()
	}
}

impl WatchTile {
	/// Starts a tile with the tile budget.
	#[must_use]
	pub fn new(id: impl Into<String>, content: Element) -> Self {
		Self {
			id: id.into(),
			content,
			refresh: RefreshPolicy::QuarterHour,
			budget: WatchBudget::tile(),
		}
	}

	/// Sets how often the host may refresh.
	#[must_use]
	pub fn refresh(mut self, refresh: RefreshPolicy) -> Self {
		self.refresh = refresh;
		self
	}

	/// Overrides the tile budget.
	#[must_use]
	pub fn budget(mut self, budget: WatchBudget) -> Self {
		self.budget = budget;
		self
	}

	/// Stable id.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Content tree.
	#[must_use]
	pub fn content(&self) -> &Element {
		&self.content
	}

	/// Refresh policy.
	#[must_use]
	pub fn refresh_policy(&self) -> RefreshPolicy {
		self.refresh
	}

	/// Validates id and content.
	pub fn validate(&self) -> Result<(), WatchError> {
		validate_id(&self.id)?;
		budget::measure(&self.content, self.budget).map(|_| ())
	}
}

/// A Smart Stack / glance surface: a tiny preview and a slightly larger body.
#[derive(Clone)]
pub struct WatchGlance {
	id: String,
	preview: Element,
	content: Element,
	budget: WatchBudget,
}

impl std::fmt::Debug for WatchGlance {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WatchGlance")
			.field("id", &self.id)
			.field("budget", &self.budget)
			.finish_non_exhaustive()
	}
}

impl WatchGlance {
	/// Starts a glance with the glance budget for both trees.
	#[must_use]
	pub fn new(id: impl Into<String>, preview: Element, content: Element) -> Self {
		Self {
			id: id.into(),
			preview,
			content,
			budget: WatchBudget::glance(),
		}
	}

	/// Overrides the glance budget.
	#[must_use]
	pub fn budget(mut self, budget: WatchBudget) -> Self {
		self.budget = budget;
		self
	}

	/// Stable id.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Compact preview.
	#[must_use]
	pub fn preview(&self) -> &Element {
		&self.preview
	}

	/// Expanded glance body.
	#[must_use]
	pub fn content(&self) -> &Element {
		&self.content
	}

	/// Validates id and both trees.
	pub fn validate(&self) -> Result<(), WatchError> {
		validate_id(&self.id)?;
		budget::measure(&self.preview, self.budget)?;
		budget::measure(&self.content, self.budget).map(|_| ())
	}
}

/// A face complication: the smallest ambient slot.
#[derive(Clone)]
pub struct WatchComplication {
	id: String,
	family: ComplicationFamily,
	content: Element,
	budget: WatchBudget,
}

impl std::fmt::Debug for WatchComplication {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WatchComplication")
			.field("id", &self.id)
			.field("family", &self.family)
			.field("budget", &self.budget)
			.finish_non_exhaustive()
	}
}

impl WatchComplication {
	/// Starts a complication with the complication budget.
	#[must_use]
	pub fn new(id: impl Into<String>, family: ComplicationFamily, content: Element) -> Self {
		Self {
			id: id.into(),
			family,
			content,
			budget: WatchBudget::complication(),
		}
	}

	/// Overrides the complication budget.
	#[must_use]
	pub fn budget(mut self, budget: WatchBudget) -> Self {
		self.budget = budget;
		self
	}

	/// Stable id.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Slot family.
	#[must_use]
	pub fn family(&self) -> ComplicationFamily {
		self.family
	}

	/// Content tree.
	#[must_use]
	pub fn content(&self) -> &Element {
		&self.content
	}

	/// Validates id and content.
	pub fn validate(&self) -> Result<(), WatchError> {
		validate_id(&self.id)?;
		budget::measure(&self.content, self.budget).map(|_| ())
	}
}

/// Any ambient surface a watch app can publish beside its pages.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // each variant owns an Element tree by design
pub enum WatchAmbient {
	/// Wear OS tile / watchOS accessory.
	Tile(WatchTile),
	/// Smart Stack / glance.
	Glance(WatchGlance),
	/// Face complication.
	Complication(WatchComplication),
}

impl WatchAmbient {
	/// Stable id.
	#[must_use]
	pub fn id(&self) -> &str {
		match self {
			Self::Tile(tile) => tile.id(),
			Self::Glance(glance) => glance.id(),
			Self::Complication(complication) => complication.id(),
		}
	}

	/// Validates the ambient surface.
	pub fn validate(&self) -> Result<(), WatchError> {
		match self {
			Self::Tile(tile) => tile.validate(),
			Self::Glance(glance) => glance.validate(),
			Self::Complication(complication) => complication.validate(),
		}
	}
}
