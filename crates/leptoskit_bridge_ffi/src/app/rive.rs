//! Synchronous native image callbacks; frames never cross guest memory or JSON.

use std::ffi::c_void;
use std::time::Duration;

use leptoskit_native::RiveEnvironment;
use leptoskit_native::RiveError;
use leptoskit_native::RiveFrame;
use leptoskit_native::RivePresenter;
use leptoskit_yoga::NodeId;

use super::App;

type Visible = unsafe extern "C" fn(*mut c_void, usize) -> i32;
type Present = unsafe extern "C" fn(*mut c_void, usize, u32, u32, *const u8, usize) -> i32;

struct Presenter {
	context: *mut c_void,
	visible: Visible,
	present: Present,
	failed: bool,
}

impl RivePresenter for Presenter {
	fn visible(&mut self, id: NodeId) -> bool {
		if self.failed {
			return false;
		}

		// SAFETY: The host lends this callback/context for the synchronous
		// tick.
		let status = unsafe { (self.visible)(self.context, id.index()) };

		if status != 0 && status != 1 {
			self.failed = true;
		}

		status == 1
	}

	fn present(&mut self, id: NodeId, frame: &RiveFrame<'_>) -> Result<(), RiveError> {
		if !self.failed {
			// SAFETY: The callback must synchronously copy/upload, must not
			// retain the borrowed pixels, and must not reenter any function on
			// this App.
			let status = unsafe {
				(self.present)(
					self.context,
					id.index(),
					frame.width(),
					frame.height(),
					frame.bytes().as_ptr(),
					frame.bytes().len(),
				)
			};

			if status == 1 {
				return Ok(());
			}
		}

		self.failed = true;
		Err(RiveError::Runtime(
			"native Rive image adapter failed".into(),
		))
	}
}

/// Whether this application owns Rive widgets, including paused scenes.
///
/// # Safety
/// `app` is a live application, borrowed on its owning UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_has_rive(app: *const App) -> i32 {
	// SAFETY: The embedding host supplies a live UI-thread application.
	unsafe { app.as_ref() }.map_or(0, |app| {
		i32::from(
			app.host
				.as_ref()
				.is_some_and(leptoskit_wasm::WasmHost::has_rive_players),
		)
	})
}

/// Draws native scenes without invoking the guest. Returns 1 while another
/// display callback is needed, 0 when idle, or -1 with a runtime diagnostic.
/// Flags: bit 0 = active visible window, bit 1 = reduced motion.
///
/// # Safety
/// `app` is exclusively borrowed on its UI thread. Callback pointers and
/// context remain valid for this call. Callbacks neither retain pixel pointers
/// nor reenter the app. `present` returns 1 for success; `visible` returns 0 or
/// 1; any other callback result reports an adapter failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_app_rive_tick(
	app: *mut App,
	elapsed: f64,
	scale: f32,
	flags: u32,
	context: *mut c_void,
	visible: Option<Visible>,
	present: Option<Present>,
) -> i32 {
	// SAFETY: The embedding host lends its application exclusively for this
	// tick.
	let Some(app) = (unsafe { app.as_mut() }) else {
		return -1;
	};
	let (Some(visible), Some(present)) = (visible, present) else {
		app.error = "native Rive callbacks are required".into();

		return -1;
	};

	if !elapsed.is_finite()
		|| elapsed < 0.0
		|| !scale.is_finite()
		|| scale <= 0.0
		|| flags & !3 != 0
	{
		app.error = "invalid native Rive display environment".into();

		return -1;
	}
	let mut presenter = Presenter {
		context,
		visible,
		present,
		failed: false,
	};
	let result = app.host_mut().tick_rive_with(
		Duration::from_secs_f64(elapsed.min(0.1)),
		RiveEnvironment {
			scale,
			active: flags & 1 != 0,
			reduced_motion: flags & 2 != 0,
		},
		&mut presenter,
	);

	match result {
		Ok(pending) if !presenter.failed => {
			app.error.clear();
			i32::from(pending)
		}

		Ok(_) => {
			app.error = "native Rive visibility adapter failed".into();
			-1
		}
		Err(error) => {
			app.error = error.to_string();
			-1
		}
	}
}
