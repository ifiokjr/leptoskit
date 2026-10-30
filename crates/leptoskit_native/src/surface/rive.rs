//! Host-owned scenes, independent of guest entry points and reconciliation.

use std::collections::BTreeMap;
use std::time::Duration;

use leptoskit_rive::Error;
use leptoskit_rive::Playback;
use leptoskit_rive::native::Frame;
use leptoskit_rive::native::MAX_FRAME_PIXELS;
use leptoskit_rive::native::Options;
use leptoskit_rive::native::Player;
use leptoskit_yoga::NodeId;

use super::NativeSurface;
use super::ShadowNode;
use crate::Mutation;
use crate::MutationList;
use crate::NativeBackend;
use crate::NativeKind;
use crate::NativeProps;

/// Window state supplied on activation, preference changes, and display ticks.
#[derive(Debug, Clone, Copy)]
pub struct RiveEnvironment {
	/// Physical pixels per logical layout unit, clamped to two.
	pub scale: f32,
	/// Whether the application window is visible and active.
	pub active: bool,
	/// The platform's current reduced-motion preference.
	pub reduced_motion: bool,
}

impl Default for RiveEnvironment {
	fn default() -> Self {
		Self {
			scale: 1.0,
			active: true,
			reduced_motion: false,
		}
	}
}

/// A synchronous native bitmap presenter. It may copy or upload the borrowed
/// frame, but cannot retain its pointer or reenter the surface during a
/// callback.
pub trait RivePresenter {
	/// Actual widget visibility, including clipping and native scroll
	/// positions.
	fn visible(&mut self, id: NodeId) -> bool;
	/// Uploads a frame to the existing image widget. Surface/window styling and
	/// accessibility remain those of the original Rive element.
	fn present(&mut self, id: NodeId, frame: &Frame<'_>) -> Result<(), Error>;
}

struct BackendPresenter<'a, B>(&'a mut B);
impl<B: NativeBackend> RivePresenter for BackendPresenter<'_, B> {
	fn visible(&mut self, id: NodeId) -> bool {
		self.0.rive_visible(id)
	}

	fn present(&mut self, id: NodeId, frame: &Frame<'_>) -> Result<(), Error> {
		if self.0.present_rive(id, frame) {
			Ok(())
		} else {
			Err(Error::Runtime(
				"native backend rejected Rive bitmap presentation".into(),
			))
		}
	}
}

struct Entry {
	content: Playback,
	player: Result<Player, Error>,
	running: bool,
}

#[derive(Default)]
pub(super) struct Players {
	entries: BTreeMap<usize, (NodeId, Entry)>,
	error: Option<Error>,
}

impl Players {
	fn update(&mut self, id: NodeId, props: &NativeProps) {
		let Some(content) = props.rive.as_deref() else {
			return;
		};
		let running = props.running == Some(true);

		if let Some((_, previous)) = self.entries.get_mut(&id.index()) {
			if previous.content == *content && previous.running == running {
				return;
			}

			if previous.content.same_scene(content)
				&& let Ok(player) = &mut previous.player
			{
				for (name, update) in content.inputs() {
					if previous.content.inputs().get(name) != Some(update)
						&& let Err(error) = player.validate_input(name, update.value)
					{
						self.error = Some(error);
						return;
					}
				}
				for (name, update) in content.inputs() {
					if previous.content.inputs().get(name) != Some(update)
						&& let Err(error) = player.set_input(name, update.value)
					{
						self.error = Some(error);
						return;
					}
				}

				if previous.running != running {
					if running {
						player.play();
					} else {
						player.pause();
					}
				}
				previous.content = content.clone();
				previous.running = running;
				return;
			}
		}

		if !self.entries.contains_key(&id.index()) && self.entries.len() >= 16 {
			self.error = Some(Error::Runtime(
				"a native surface supports at most 16 Rive players".into(),
			));
			return;
		}
		let player = Player::new(
			content.asset(),
			&Options {
				artboard: content.artboard_name().cloned(),
				scene: content.selected_scene().clone(),
				autoplay: running,
			},
		)
		.and_then(|mut player| {
			for (name, update) in content.inputs() {
				player.set_input(name, update.value)?;
			}
			Ok(player)
		});

		if let Err(error) = &player {
			self.error = Some(error.clone());
		}
		self.entries.insert(
			id.index(),
			(
				id,
				Entry {
					content: content.clone(),
					player,
					running,
				},
			),
		);
	}

	fn tick(
		&mut self,
		shadows: &[ShadowNode],
		frames: &[Option<super::Frame>],
		elapsed: Duration,
		environment: RiveEnvironment,
		presenter: &mut impl RivePresenter,
	) -> Result<bool, Error> {
		if let Some(error) = self.error.take() {
			return Err(error);
		}
		if !environment.scale.is_finite() || environment.scale <= 0.0 {
			return Err(Error::InvalidFrame);
		}
		let budget = MAX_FRAME_PIXELS.min(8_388_608 / self.entries.len().max(1) as u64);
		let mut needs_frame = false;

		for (id, entry) in self.entries.values_mut() {
			let Ok(player) = &mut entry.player else {
				continue;
			};
			let layout = frames[id.index()];
			let mut visible =
				environment.active && layout.is_some_and(|f| f.width > 0.0 && f.height > 0.0);
			let mut ancestor = Some(*id);

			while let Some(node) = ancestor {
				let shadow = &shadows[node.index()];
				visible &= shadow.props.hidden != Some(true) && shadow.props.opacity != Some(0.0);
				ancestor = shadow.parent;
			}
			visible &= presenter.visible(*id);
			player.set_visible(visible);
			player.set_reduced_motion(environment.reduced_motion);

			if !visible {
				continue;
			}
			let layout = layout.ok_or(Error::InvalidFrame)?;
			let (width, height) =
				backing_size(layout.width, layout.height, environment.scale, budget)?;
			let result = player.render(elapsed, width, height).and_then(|frame| {
				if let Some(frame) = frame {
					presenter.present(*id, &frame)?;
				}
				Ok(())
			});

			if let Err(error) = result {
				entry.player = Err(error.clone());
				return Err(error);
			}
			needs_frame |= player.needs_frame();
		}
		Ok(needs_frame)
	}
}

fn backing_size(width: f32, height: f32, scale: f32, budget: u64) -> Result<(u32, u32), Error> {
	if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
		return Err(Error::InvalidFrame);
	}
	let (width, height) = (f64::from(width), f64::from(height));
	let scale = f64::from(scale.min(2.0))
		.min(4096.0 / width)
		.min(4096.0 / height)
		.min((budget as f64 / (width * height)).sqrt());
	Ok((
		(width * scale).floor().max(1.0) as u32,
		(height * scale).floor().max(1.0) as u32,
	))
}

impl<B: NativeBackend> NativeSurface<B> {
	/// Whether the surface owns a Rive scene, including a paused scene.
	pub fn has_rive_players(&self) -> bool {
		!self.rive.entries.is_empty() || self.rive.error.is_some()
	}

	/// Drops every Rive player on the calling thread. Players must be
	/// released on the thread that created them, so a host that rendered on
	/// a thread of its own calls this there before handing the surface to
	/// another thread to drop.
	pub fn release_rive_players(&mut self) {
		self.rive.entries.clear();
	}

	pub(super) fn sync_rive(&mut self, mutations: MutationList) -> MutationList {
		let mut output = MutationList::new();

		for mut mutation in mutations.into_mutations() {
			match &mut mutation {
				Mutation::Create { id, kind, props } if *kind == NativeKind::Rive => {
					self.rive.update(*id, props);
					// Backends need only image styling. Retain the semantic
					// kind, but never serialize encoded assets a second time
					// into native UI JSON.
					props.rive = None;
				}
				Mutation::UpdateProps { id, props }
					if self.shadows[id.index()].kind == NativeKind::Rive =>
				{
					self.rive.update(*id, props);
					props.rive = None;
				}
				Mutation::Delete { id } => {
					self.rive.entries.remove(&id.index());
				}
				_ => {}
			}
			output.push(mutation);
		}
		output
	}

	/// Whether an activation/commit needs a display callback. A paused, hidden,
	/// reduced-motion, settled, or failed player does not keep a timer running.
	pub fn rive_needs_frame(&self) -> bool {
		self.rive.error.is_some()
			|| self
				.rive
				.entries
				.values()
				.any(|(_, entry)| entry.player.as_ref().is_ok_and(Player::needs_frame))
	}

	/// Draws through the Rust widget backend. Returns whether another display
	/// callback is needed. Call once after activation or a viewport/preference
	/// change, then schedule only while true. This never enters the Wasm guest.
	pub fn tick_rive(
		&mut self,
		elapsed: Duration,
		environment: RiveEnvironment,
	) -> Result<bool, Error> {
		self.rive.tick(
			&self.shadows,
			&self.frames,
			elapsed,
			environment,
			&mut BackendPresenter(&mut self.backend),
		)
	}

	/// Draws through a Swift, Kotlin, or ArkUI embedding's synchronous
	/// presenter.
	pub fn tick_rive_with(
		&mut self,
		elapsed: Duration,
		environment: RiveEnvironment,
		presenter: &mut impl RivePresenter,
	) -> Result<bool, Error> {
		self.rive
			.tick(&self.shadows, &self.frames, elapsed, environment, presenter)
	}
}
