//! Thread-confined native playback with reusable premultiplied RGBA frames.
//!
//! The pinned C++ runtime has no network loaders, scripts, audio, or native
//! input listeners. Only use trusted files. The encoded-size and raster bounds
//! do not sandbox the upstream C++ parser or limit all of its decoded object
//! memory.

use std::time::Duration;

use leptoskit_rive_runtime as _;

use crate::Asset;
use crate::Error;
use crate::Input;
use crate::Name;
use crate::Scene;

// The only unsafe code in this integration is the private C ABI boundary.
#[allow(unsafe_code)]
mod ffi;
mod raster;

pub use crate::Frame;
pub use crate::MAX_FRAME_PIXELS;

/// Selection and initial playback intent. Players begin paused by default.
#[derive(Debug, Clone, Default)]
pub struct Options {
	/// The file's default artboard unless a name is supplied.
	pub artboard: Option<Name>,
	/// A timeline or state machine; default selects the first timeline.
	pub scene: Scene,
	/// Request playback after the first pose. Visibility and reduced motion
	/// win.
	pub autoplay: bool,
}

/// One owned native player. Keep it on its creating thread and drop it to
/// release its scene, parsed file, path cache, clip masks, and backing buffer.
///
/// Hosts call [`Self::render`] from their display callback only while
/// [`Self::needs_frame`] is true, and stop their callback while hidden or
/// inactive. Never keep a platform timer alive solely to poll a paused player.
///
/// ```compile_fail
/// use leptoskit_rive::native::Player;
/// fn move_to_another_thread(player: Player) {
///     std::thread::spawn(move || drop(player));
/// }
/// ```
///
/// ```compile_fail
/// use leptoskit_rive::native::Player;
/// use std::time::Duration;
/// fn overwrite_a_borrowed_frame(mut player: Player) {
///     let frame = player.render(Duration::ZERO, 128, 128).unwrap().unwrap();
///     player.render(Duration::ZERO, 128, 128).unwrap();
///     println!("{}", frame.bytes().len());
/// }
/// ```
pub struct Player {
	runtime: ffi::Runtime,
	_asset: Asset,
	requested: bool,
	visible: bool,
	reduced_motion: bool,
	frame_state: FrameState,
	size: (u32, u32),
	error: Option<Error>,
}

#[derive(Clone, Copy)]
enum FrameState {
	Dirty,
	Animating,
	Settled,
}

impl std::fmt::Debug for Player {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Player")
			.field("requested", &self.requested)
			.field("visible", &self.visible)
			.field("size", &self.size)
			.finish_non_exhaustive()
	}
}

impl Player {
	/// Imports a trusted file once, with no asset downloads or filesystem
	/// access.
	pub fn new(asset: &Asset, options: &Options) -> Result<Self, Error> {
		Ok(Self {
			runtime: ffi::Runtime::new(asset, options)?,
			_asset: asset.clone(),
			requested: options.autoplay,
			visible: true,
			reduced_motion: false,
			frame_state: FrameState::Dirty,
			size: (0, 0),
			error: None,
		})
	}

	/// Requests playback; it resumes only while visible and motion is allowed.
	pub fn play(&mut self) {
		self.requested = true;
		if matches!(self.frame_state, FrameState::Settled) {
			self.frame_state = FrameState::Animating;
		}
	}

	/// Stops playback without losing the current pose.
	pub fn pause(&mut self) {
		self.requested = false;
	}

	/// Supplies effective visibility, including window activity and ancestors.
	pub fn set_visible(&mut self, visible: bool) {
		self.visible = visible;
	}

	/// Supplies the platform's current reduced-motion preference.
	pub fn set_reduced_motion(&mut self, reduced: bool) {
		self.reduced_motion = reduced;
	}

	/// Whether the host should request another display callback.
	pub fn needs_frame(&self) -> bool {
		self.error.is_none()
			&& self.visible
			&& (matches!(self.frame_state, FrameState::Dirty)
				|| (self.requested
					&& !self.reduced_motion
					&& matches!(self.frame_state, FrameState::Animating)))
	}

	/// Checks a typed input without changing its value or firing a trigger.
	/// Hosts can validate an entire controlled update before applying any of
	/// it.
	pub fn validate_input(&mut self, name: &Name, input: Input) -> Result<(), Error> {
		input.validate()?;
		if let Some(error) = &self.error {
			return Err(error.clone());
		}
		self.runtime.input(name, input, false)
	}

	/// Updates a named typed state-machine input. Missing inputs and mismatched
	/// types are errors; the next visible frame applies the update while
	/// paused.
	pub fn set_input(&mut self, name: &Name, input: Input) -> Result<(), Error> {
		input.validate()?;
		if let Some(error) = &self.error {
			return Err(error.clone());
		}
		self.runtime.input(name, input, true)?;
		self.frame_state = FrameState::Dirty;
		Ok(())
	}

	/// Advances and draws at most one frame, reusing allocations while its size
	/// is unchanged. `None` means no work or upload is needed. Elapsed time is
	/// capped at 100 ms so returning from background cannot fast-forward a
	/// scene.
	///
	/// The first call draws the initial pose, including for a paused player.
	pub fn render(
		&mut self,
		elapsed: Duration,
		width: u32,
		height: u32,
	) -> Result<Option<Frame<'_>>, Error> {
		if let Some(error) = &self.error {
			return Err(error.clone());
		}
		if width == 0
			|| height == 0
			|| width > 4096
			|| height > 4096
			|| u64::from(width) * u64::from(height) > MAX_FRAME_PIXELS
		{
			return Err(Error::InvalidFrame);
		}

		if self.size != (width, height) {
			self.frame_state = FrameState::Dirty;
		}

		if !self.needs_frame() {
			return Ok(None);
		}

		let elapsed = if self.requested && !self.reduced_motion && self.size != (0, 0) {
			elapsed.min(Duration::from_millis(100)).as_secs_f32()
		} else {
			0.0
		};
		self.frame_state = match self.runtime.render(elapsed, width, height) {
			Ok(true) => FrameState::Animating,
			Ok(false) => FrameState::Settled,
			Err(error) => {
				self.error = Some(error.clone());
				return Err(error);
			}
		};
		self.size = (width, height);
		let pixels = self.runtime.pixels();
		Ok(Some(Frame {
			width,
			height,
			bytes: pixels.data(),
		}))
	}
}
