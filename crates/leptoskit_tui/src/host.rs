//! The host loop: events in, dispatch, repaint.

use std::time::Duration;
use std::time::Instant;

use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_yoga::NodeId;

use crate::backend::TuiBackend;
use crate::buffer::Buffer;
use crate::input::Key;
use crate::input::KeyCode;

/// The duration between animation ticks.
const TICK: Duration = Duration::from_millis(80);

/// What an application provides the host loop: its backend, a dispatch hop
/// through its own state, and a viewport resize.
///
/// A direct app wraps a [`NativeSurface`](leptoskit_native::NativeSurface); a
/// generated terminal runner wraps its wasm host. The host owns the terminal.
pub trait TuiApplication {
	/// The backend the surface commits into.
	fn backend(&mut self) -> &mut TuiBackend;

	/// Dispatches one shared event and commits whatever re-render follows.
	fn dispatch(&mut self, node: NodeId, event: &NativeEvent);

	/// Resizes to a terminal size in cells.
	fn resize(&mut self, cols: u16, rows: u16);

	/// Commits the first tree; called once after the first resize.
	fn mount(&mut self) {}

	/// Handles an app-wide shortcut before focus routing sees the key, such
	/// as ctrl+p opening a command palette or escape closing a dialog.
	/// Returns whether the key was handled; unhandled keys route to the
	/// focused widget as usual.
	fn shortcut(&mut self, _key: Key) -> bool {
		false
	}
}

/// Where painted frames go: a terminal, or a test recorder.
pub trait OutputSink {
	/// The terminal size in cells.
	///
	/// # Errors
	///
	/// Returns an error when the size cannot be read.
	fn size(&mut self) -> Result<(u16, u16), crate::TuiError>;

	/// Draws one frame, reporting whether anything changed.
	///
	/// # Errors
	///
	/// Returns an error when the frame cannot be written.
	fn draw(&mut self, next: &Buffer) -> Result<bool, crate::TuiError>;
}

/// One decoded terminal event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceEvent {
	/// A key press.
	Key(Key),
	/// A mouse event.
	Mouse(crate::input::Mouse),
	/// The terminal resized.
	Resize(u16, u16),
}

/// Where terminal events come from.
pub trait EventSource {
	/// Waits up to `timeout` for an event.
	///
	/// # Errors
	///
	/// Returns an error when the source failed.
	fn poll(&mut self, timeout: Duration) -> Result<bool, crate::TuiError>;

	/// Reads the event a successful poll announced.
	///
	/// # Errors
	///
	/// Returns an error when the source failed. `Ok(None)` ends the host
	/// loop, which scripted test sources use to finish a session.
	fn read(&mut self) -> Result<Option<SourceEvent>, crate::TuiError>;

	/// Whether the source can still produce events. A source that is done
	/// ends the host loop; real terminals never are.
	fn done(&mut self) -> bool {
		false
	}
}

/// Why a host loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
	/// The configured exit key was pressed.
	ExitKey,
	/// The event source ended.
	SourceEnded,
}

/// Terminal host options.
#[derive(Debug, Clone)]
pub struct TuiOptions {
	/// The terminal window title to set while running.
	pub title: String,
	/// Whether to run on the alternate screen buffer, restored on exit.
	pub alternate_screen: bool,
	/// Whether to capture the mouse for clicks and wheel scrolling.
	pub mouse: bool,
	/// Whether the host's color scheme is dark, which the backend paints
	/// defaults for.
	pub dark: bool,
	/// The animation tick, driving spinners and indeterminate progress.
	pub tick: Duration,
	/// The key that quits the host loop, pressed with control by default.
	pub exit: Option<Key>,
}

impl Default for TuiOptions {
	fn default() -> Self {
		Self {
			title: String::new(),
			alternate_screen: true,
			mouse: true,
			dark: true,
			tick: TICK,
			exit: Some(Key::controlled(KeyCode::Char('c'))),
		}
	}
}

/// Runs an application against an event source and an output sink until the
/// exit key or the source ends.
///
/// Every input routes through [`TuiBackend::handle_key`] or
/// [`TuiBackend::handle_mouse`], every returned event dispatches through the
/// application, and the frame repaints afterwards. Ticks advance animations
/// between input.
#[derive(Debug, Clone)]
pub struct TuiHost {
	options: TuiOptions,
	size: (u16, u16),
}

impl TuiHost {
	/// A host for the options.
	pub const fn new(options: TuiOptions) -> Self {
		Self {
			options,
			size: (1, 1),
		}
	}

	/// The host's options.
	pub const fn options(&self) -> &TuiOptions {
		&self.options
	}

	/// Runs the loop.
	///
	/// # Errors
	///
	/// Returns the source's or the sink's errors.
	pub fn run(
		&mut self,
		app: &mut dyn TuiApplication,
		source: &mut dyn EventSource,
		out: &mut dyn OutputSink,
	) -> Result<Exit, crate::TuiError> {
		// A terminal can briefly report a zero dimension during startup;
		// every frame needs at least one cell.
		self.size = clamped(out.size()?);
		app.resize(self.size.0, self.size.1);
		app.backend().set_dark_mode(self.options.dark);
		app.mount();
		self.repaint(app, out)?;

		let mut last_tick = Instant::now();

		loop {
			if source.done() {
				return Ok(Exit::SourceEnded);
			}

			let timeout = self.options.tick.saturating_sub(last_tick.elapsed());

			if source.poll(timeout)? {
				match source.read()? {
					None => return Ok(Exit::SourceEnded),
					Some(SourceEvent::Key(key)) => {
						if self.is_exit(key) {
							return Ok(Exit::ExitKey);
						}

						if app.shortcut(key) {
							self.repaint(app, out)?;
							continue;
						}

						for dispatch in app.backend().handle_key(&key) {
							app.dispatch(dispatch.node, &dispatch.event);
						}
					}
					Some(SourceEvent::Mouse(mouse)) => {
						for dispatch in app.backend().handle_mouse(&mouse) {
							app.dispatch(dispatch.node, &dispatch.event);
						}
					}
					Some(SourceEvent::Resize(cols, rows)) => {
						self.size = clamped((cols, rows));
						app.resize(self.size.0, self.size.1);
					}
				}

				self.repaint(app, out)?;
			}

			if last_tick.elapsed() >= self.options.tick {
				last_tick = Instant::now();

				if app.backend().advance_tick() {
					self.repaint(app, out)?;
				}
			}
		}
	}

	fn is_exit(&self, key: Key) -> bool {
		self.options.exit.is_some_and(|exit| {
			exit.code == key.code && exit.ctrl == key.ctrl && exit.alt == key.alt
		})
	}

	fn repaint(
		&self,
		app: &mut dyn TuiApplication,
		out: &mut dyn OutputSink,
	) -> Result<(), crate::TuiError> {
		let buffer = app.backend().render(self.size.0, self.size.1);
		out.draw(&buffer)?;
		Ok(())
	}
}

/// A terminal size with at least one cell of each dimension.
fn clamped(size: (u16, u16)) -> (u16, u16) {
	(size.0.max(1), size.1.max(1))
}
