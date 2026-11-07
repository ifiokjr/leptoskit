//! A watch application: pages plus ambient surfaces.

use std::collections::BTreeSet;

use crate::WatchAmbient;
use crate::WatchError;
use crate::WatchKind;
use crate::WatchNavigation;
use crate::WatchPage;

/// A Wear OS or watchOS application described as watch surfaces, not phone
/// routes.
///
/// Build one app per watch kind. Share content helpers between kinds if you
/// want, but keep the `WatchApp` itself explicit about Wear OS vs watchOS so
/// entitlements, tiles, and complications stay honest.
#[derive(Debug, Clone)]
pub struct WatchApp {
	id: String,
	kind: WatchKind,
	navigation: WatchNavigation,
	pages: Vec<WatchPage>,
	ambients: Vec<WatchAmbient>,
}

impl WatchApp {
	/// Starts an empty app for `kind`.
	#[must_use]
	pub fn new(id: impl Into<String>, kind: WatchKind) -> Self {
		Self {
			id: id.into(),
			kind,
			navigation: WatchNavigation::Single,
			pages: Vec::new(),
			ambients: Vec::new(),
		}
	}

	/// Sets the navigation model.
	#[must_use]
	pub fn navigation(mut self, navigation: WatchNavigation) -> Self {
		self.navigation = navigation;
		self
	}

	/// Appends a page.
	#[must_use]
	pub fn page(mut self, page: WatchPage) -> Self {
		self.pages.push(page);
		self
	}

	/// Appends an ambient surface.
	#[must_use]
	pub fn ambient(mut self, ambient: WatchAmbient) -> Self {
		self.ambients.push(ambient);
		self
	}

	/// Appends a tile.
	#[must_use]
	pub fn tile(self, tile: crate::WatchTile) -> Self {
		self.ambient(WatchAmbient::Tile(tile))
	}

	/// Appends a glance.
	#[must_use]
	pub fn glance(self, glance: crate::WatchGlance) -> Self {
		self.ambient(WatchAmbient::Glance(glance))
	}

	/// Appends a complication.
	#[must_use]
	pub fn complication(self, complication: crate::WatchComplication) -> Self {
		self.ambient(WatchAmbient::Complication(complication))
	}

	/// App id.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Wear OS or watchOS.
	#[must_use]
	pub fn kind(&self) -> WatchKind {
		self.kind
	}

	/// Navigation model.
	#[must_use]
	pub fn navigation_model(&self) -> WatchNavigation {
		self.navigation
	}

	/// Pages in declaration order.
	#[must_use]
	pub fn pages(&self) -> &[WatchPage] {
		&self.pages
	}

	/// Ambient surfaces in declaration order.
	#[must_use]
	pub fn ambients(&self) -> &[WatchAmbient] {
		&self.ambients
	}

	/// Validates ids, navigation, and every surface budget.
	pub fn validate(&self) -> Result<(), WatchError> {
		crate::page::validate_id(&self.id)?;

		if matches!(
			self.navigation,
			WatchNavigation::Stack | WatchNavigation::Pager | WatchNavigation::ListDetail
		) && self.pages.is_empty()
		{
			return Err(WatchError::MissingPage(self.navigation));
		}

		let mut seen = BTreeSet::new();
		for page in &self.pages {
			page.validate()?;
			if !seen.insert(page.id().to_owned()) {
				return Err(WatchError::DuplicateId(page.id().to_owned()));
			}
		}
		for ambient in &self.ambients {
			ambient.validate()?;
			if !seen.insert(ambient.id().to_owned()) {
				return Err(WatchError::DuplicateId(ambient.id().to_owned()));
			}
		}
		Ok(())
	}
}
