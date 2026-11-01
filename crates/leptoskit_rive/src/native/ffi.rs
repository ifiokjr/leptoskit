//! Private synchronous ABI. C++ owns scenes; Rust owns the raster context.

use std::ffi::c_char;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use super::Options;
use super::raster::Raster;
use crate::Asset;
use crate::Error;
use crate::Input;
use crate::Name;
use crate::Scene;

#[repr(C)]
pub(super) struct PathView {
	pub id: u64,
	pub revision: u64,
	pub verbs: *const u8,
	pub verb_count: usize,
	pub points: *const f32,
	pub point_count: usize,
	pub fill_rule: u32,
}

#[repr(C)]
pub(super) struct PaintView {
	pub color: u32,
	pub style: u32,
	pub thickness: f32,
	pub join: u32,
	pub cap: u32,
	pub blend: u32,
	pub gradient: u32,
	pub coordinates: [f32; 5],
	pub colors: *const u32,
	pub stops: *const f32,
	pub stop_count: usize,
	pub shader_matrix: [f32; 6],
}

#[repr(C)]
struct Command {
	operation: u32,
	matrix: [f32; 6],
	path: *const PathView,
	paint: *const PaintView,
	opacity: f32,
	released_path: u64,
}

unsafe extern "C" {
	fn lk_rive_new(
		bytes: *const u8,
		size: usize,
		board: *const c_char,
		board_len: usize,
		scene_kind: u32,
		scene: *const c_char,
		scene_len: usize,
		context: *mut c_void,
		emit: unsafe extern "C" fn(*mut c_void, *const Command),
		error: *mut u32,
	) -> *mut c_void;
	fn lk_rive_delete(player: *mut c_void);
	fn lk_rive_draw(
		player: *mut c_void,
		elapsed: f32,
		width: u32,
		height: u32,
		more: *mut u32,
	) -> u32;
	fn lk_rive_input(
		player: *mut c_void,
		name: *const c_char,
		len: usize,
		kind: u32,
		value: f32,
		apply: bool,
	) -> u32;
}

fn status(code: u32) -> Result<(), Error> {
	match code {
		0 => Ok(()),
		1 => {
			Err(Error::Runtime(
				"malformed native file or artboard bounds".into(),
			))
		}
		2 => Err(Error::Runtime("native artboard or scene not found".into())),
		3 => Err(Error::UnsupportedFeature),
		4 => {
			Err(Error::Runtime(
				"native state-machine input missing, duplicated, or type mismatch".into(),
			))
		}
		_ => {
			Err(Error::Runtime(
				"native runtime allocation or execution failure".into(),
			))
		}
	}
}

// C++ retains this pointer. Own it through a raw allocation so later Rust
// borrows and moves cannot re-tag a Box pointee behind the foreign pointer.
struct RasterContext(NonNull<Raster>);

impl RasterContext {
	fn new() -> Result<Self, Error> {
		Ok(Self(NonNull::from(Box::leak(Box::new(Raster::new()?)))))
	}

	fn as_ptr(&self) -> *mut Raster {
		self.0.as_ptr()
	}

	fn raster(&self) -> &Raster {
		// SAFETY: The allocation is live, and no C++ callback runs while
		// these pixels are borrowed from the runtime.
		unsafe { self.0.as_ref() }
	}

	fn raster_mut(&mut self) -> &mut Raster {
		// SAFETY: Rust accesses and C++ callbacks are strictly sequential.
		// This borrow ends before the next foreign call.
		unsafe { self.0.as_mut() }
	}
}

impl Drop for RasterContext {
	fn drop(&mut self) {
		// SAFETY: This owner reclaims its allocation exactly once, after the
		// C++ runtime and all path-release callbacks have finished.
		unsafe {
			drop(Box::from_raw(self.0.as_ptr()));
		}
	}
}

pub(super) struct Runtime {
	raw: NonNull<c_void>,
	raster: RasterContext,
	// Upstream objects and callbacks are never shared between threads.
	_thread: PhantomData<Rc<()>>,
}

impl Runtime {
	pub fn new(asset: &Asset, options: &Options) -> Result<Self, Error> {
		let board = options.artboard.as_ref().map_or("", Name::as_str);

		let (kind, scene) = match &options.scene {
			Scene::Default => (0, ""),
			Scene::Animation(name) => (1, name.as_str()),
			Scene::StateMachine(name) => (2, name.as_str()),
		};

		let raster = RasterContext::new()?;
		let mut code = 0;
		// SAFETY: Inputs stay borrowed through this call. The owned context has
		// a stable address until after C++ destruction, including failed
		// import.
		let raw = unsafe {
			lk_rive_new(
				asset.bytes().as_ptr(),
				asset.bytes().len(),
				board.as_ptr().cast(),
				board.len(),
				kind,
				scene.as_ptr().cast(),
				scene.len(),
				raster.as_ptr().cast(),
				emit,
				&raw mut code,
			)
		};
		let Some(raw) = NonNull::new(raw) else {
			status(code)?;

			return Err(Error::Runtime("native import returned no player".into()));
		};

		let runtime = Self {
			raw,
			raster,
			_thread: PhantomData,
		};
		status(code)?;
		Ok(runtime)
	}

	pub fn input(&mut self, name: &Name, input: Input, apply: bool) -> Result<(), Error> {
		let (kind, value) = match input {
			Input::Boolean(value) => (0, f32::from(u8::from(value))),
			Input::Number(value) => (1, value as f32),
			Input::Trigger => (2, 0.0),
		};

		// SAFETY: The live player is exclusively borrowed, the validated name
		// is borrowed only for this synchronous call, and numbers were checked.
		status(unsafe {
			lk_rive_input(
				self.raw.as_ptr(),
				name.as_str().as_ptr().cast(),
				name.as_str().len(),
				kind,
				value,
				apply,
			)
		})
	}

	pub fn render(&mut self, elapsed: f32, width: u32, height: u32) -> Result<bool, Error> {
		self.raster.raster_mut().begin(width, height)?;
		let mut more = 0;
		// SAFETY: The player/context remain exclusively owned. All callback
		// views borrow C++ data for the duration of that callback only.
		status(unsafe { lk_rive_draw(self.raw.as_ptr(), elapsed, width, height, &raw mut more) })?;
		self.raster.raster_mut().finish()?;
		Ok(more != 0)
	}

	pub fn pixels(&self) -> &tiny_skia::Pixmap {
		self.raster.raster().pixels()
	}
}

impl Drop for Runtime {
	fn drop(&mut self) {
		// SAFETY: Exactly one owner releases the player, before the owned
		// callback context drops. C++ also releases every cached-path identity.
		unsafe {
			lk_rive_delete(self.raw.as_ptr());
		}
	}
}

unsafe extern "C" fn emit(context: *mut c_void, command: *const Command) {
	// SAFETY: C++ calls synchronously with this stable Raster and a live
	// command.
	let (raster, command) = unsafe { (&mut *context.cast::<Raster>(), &*command) };

	if raster.failed() && command.operation != 6 {
		return;
	}

	let result = match command.operation {
		0 => raster.save(),
		1 => raster.restore(),
		2 => raster.transform(command.matrix),
		5 => raster.opacity(command.opacity),
		6 => {
			raster.release(command.released_path);
			Ok(())
		}
		3 | 4 => {
			// SAFETY: Draw and clip commands provide a live path. Counts are
			// checked before constructing any slice, including empty paths.
			let path = unsafe { &*command.path };
			if path.verb_count > 262_144 || path.point_count > 1_572_864 {
				Err(Error::Runtime("native path exceeds raster limit".into()))
			} else {
				// SAFETY: Nonempty arrays are owned by the C++ path for this
				// call.
				let verbs = if path.verb_count == 0 {
					&[]
				} else {
					unsafe { std::slice::from_raw_parts(path.verbs, path.verb_count) }
				};
				let points = if path.point_count == 0 {
					&[]
				} else {
					unsafe { std::slice::from_raw_parts(path.points, path.point_count) }
				};

				if command.operation == 3 {
					raster.clip(path, verbs, points)
				} else {
					// SAFETY: Draw provides a live paint, with bounded gradient
					// arrays.
					let paint = unsafe { &*command.paint };

					if paint.stop_count > 4096 {
						Err(Error::UnsupportedFeature)
					} else {
						let (colors, stops) = if paint.stop_count == 0 {
							(&[][..], &[][..])
						} else {
							unsafe {
								(
									std::slice::from_raw_parts(paint.colors, paint.stop_count),
									std::slice::from_raw_parts(paint.stops, paint.stop_count),
								)
							}
						};

						raster.draw(path, verbs, points, paint, colors, stops)
					}
				}
			}
		}
		_ => Err(Error::Runtime("invalid native draw command".into())),
	};

	if let Err(error) = result {
		raster.fail(error);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn path() -> PathView {
		PathView {
			id: 1,
			revision: 0,
			verbs: std::ptr::null(),
			verb_count: 0,
			points: std::ptr::null(),
			point_count: 0,
			fill_rule: 0,
		}
	}

	fn command(path: &PathView, paint: &PaintView) -> Command {
		Command {
			operation: 4,
			matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
			path,
			paint,
			opacity: 1.0,
			released_path: path.id,
		}
	}

	fn paint() -> PaintView {
		PaintView {
			color: 0xffff_0000,
			style: 1,
			thickness: 0.0,
			join: 0,
			cap: 0,
			blend: 3,
			gradient: 0,
			coordinates: [0.0; 5],
			colors: std::ptr::null(),
			stops: std::ptr::null(),
			stop_count: 0,
			shader_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
		}
	}

	fn send(raster: &mut Raster, command: &Command) {
		// SAFETY: The stack-owned raster, command, and its path/paint remain
		// live through this synchronous callback. Nonempty arrays are either
		// live slices or exceed the bounds checked before slice construction.
		unsafe { emit(std::ptr::from_mut(raster).cast(), command) };
	}

	#[test]
	fn callback_bounds_reject_arrays_before_reading_their_pointers() {
		let mut raster = Raster::new().unwrap();
		let paint = paint();
		for path in [
			PathView {
				verb_count: 262_145,
				..path()
			},
			PathView {
				point_count: 1_572_865,
				..path()
			},
		] {
			raster.begin(2, 2).unwrap();
			send(&mut raster, &command(&path, &paint));
			assert!(
				raster
					.finish()
					.unwrap_err()
					.to_string()
					.contains("path exceeds raster limit")
			);
		}
		let path = path();
		let paint = PaintView {
			stop_count: 4097,
			..paint
		};
		raster.begin(2, 2).unwrap();
		send(&mut raster, &command(&path, &paint));
		assert_eq!(raster.finish(), Err(Error::UnsupportedFeature));
	}

	#[test]
	fn empty_paths_and_terminal_callback_errors_preserve_a_valid_next_frame() {
		let mut raster = Raster::new().unwrap();
		let path = path();
		let paint = paint();
		let mut command = command(&path, &paint);
		raster.begin(2, 2).unwrap();
		send(&mut raster, &command);
		command.operation = 3;
		send(&mut raster, &command);

		assert!(raster.pixels().data().iter().all(|byte| *byte == 0));
		raster.finish().unwrap();
		command.operation = u32::MAX;
		send(&mut raster, &command);
		command.operation = 1;
		send(&mut raster, &command);
		command.operation = 6;
		send(&mut raster, &command);
		assert!(
			raster
				.finish()
				.unwrap_err()
				.to_string()
				.contains("invalid native draw command")
		);
		raster.begin(2, 2).unwrap();
		command.operation = 0;
		send(&mut raster, &command);
		command.operation = 1;
		send(&mut raster, &command);
		raster.finish().unwrap();
	}
}
