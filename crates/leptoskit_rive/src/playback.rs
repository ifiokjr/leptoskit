//! Portable descriptions; only a native host owns live runtime objects.

use std::collections::BTreeMap;

use crate::Asset;
use crate::Error;
use crate::Input;
use crate::Name;
use crate::Scene;

/// App-owned native widget content. Reconciliation retains a player until its
/// asset, artboard, or scene changes. Runtime objects never cross the Wasm ABI.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "Wire", into = "Wire")
)]
pub struct Playback {
	asset: Asset,
	artboard: Option<Name>,
	scene: Scene,
	inputs: BTreeMap<Name, Update>,
}

/// A repeated trigger needs a fresh sequence, so unrelated renders cannot
/// accidentally fire it again. Ordinary values may leave the sequence at zero.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Update {
	/// The typed input value.
	pub value: Input,
	/// App-owned update identity, changed to explicitly repeat a trigger.
	pub sequence: u64,
}

impl Playback {
	/// Describes the default artboard and its first timeline, initially paused.
	pub fn new(asset: Asset) -> Self {
		Self {
			asset,
			artboard: None,
			scene: Scene::Default,
			inputs: BTreeMap::new(),
		}
	}

	/// Chooses a named artboard at the next import.
	#[must_use]
	pub fn artboard(mut self, name: Name) -> Self {
		self.artboard = Some(name);
		self
	}

	/// Chooses exactly one timeline or state machine at the next import.
	#[must_use]
	pub fn scene(mut self, scene: Scene) -> Self {
		self.scene = scene;
		self
	}

	/// Adds or replaces one of at most 64 inputs. A changed sequence explicitly
	/// repeats a trigger; unchanged updates are not replayed during
	/// reconciliation.
	pub fn set_input(&mut self, name: Name, value: Input, sequence: u64) -> Result<(), Error> {
		value.validate()?;

		if self.inputs.len() >= 64 && !self.inputs.contains_key(&name) {
			return Err(Error::Runtime(
				"native widget exceeds 64 input bindings".into(),
			));
		}
		self.inputs.insert(name, Update { value, sequence });
		Ok(())
	}

	/// The shared encoded asset.
	pub fn asset(&self) -> &Asset {
		&self.asset
	}

	/// The selected artboard, if explicit.
	pub fn artboard_name(&self) -> Option<&Name> {
		self.artboard.as_ref()
	}

	/// The selected scene.
	pub fn selected_scene(&self) -> &Scene {
		&self.scene
	}

	/// Controlled input updates and their explicit repetition identities.
	pub fn inputs(&self) -> &BTreeMap<Name, Update> {
		&self.inputs
	}

	/// Replaces input bindings without changing the asset or scene. Validation
	/// finishes before any existing binding is removed.
	pub fn replace_inputs(&mut self, inputs: BTreeMap<Name, Update>) -> Result<(), Error> {
		if inputs.len() > 64 {
			return Err(Error::Runtime(
				"native widget exceeds 64 input bindings".into(),
			));
		}
		for update in inputs.values() {
			update.value.validate()?;
		}
		self.inputs = inputs;
		Ok(())
	}

	/// Whether an existing parsed player can be retained across this update.
	pub fn same_scene(&self, other: &Self) -> bool {
		self.asset == other.asset && self.artboard == other.artboard && self.scene == other.scene
	}
}

#[cfg(feature = "serde")]
use base64::Engine;

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct Wire {
	asset: String,
	artboard: Option<Name>,
	scene: Scene,
	inputs: BTreeMap<Name, Update>,
}

#[cfg(feature = "serde")]
impl From<Playback> for Wire {
	fn from(value: Playback) -> Self {
		Self {
			asset: base64::engine::general_purpose::STANDARD.encode(value.asset.bytes()),
			artboard: value.artboard,
			scene: value.scene,
			inputs: value.inputs,
		}
	}
}

#[cfg(feature = "serde")]
impl TryFrom<Wire> for Playback {
	type Error = Error;

	fn try_from(wire: Wire) -> Result<Self, Error> {
		// Bound decoded allocation before asking the codec to allocate. The
		// shared asset boundary checks the exact length and version afterwards.
		if wire.asset.len() > crate::MAX_ASSET_BYTES.div_ceil(3) * 4 {
			return Err(Error::AssetTooLarge);
		}
		let bytes = base64::engine::general_purpose::STANDARD
			.decode(wire.asset)
			.map_err(|_| Error::InvalidHeader)?;
		let mut playback = Self {
			asset: Asset::new(&bytes)?,
			artboard: wire.artboard,
			scene: wire.scene,
			inputs: BTreeMap::new(),
		};

		for (name, update) in wire.inputs {
			playback.set_input(name, update.value, update.sequence)?;
		}
		Ok(playback)
	}
}
