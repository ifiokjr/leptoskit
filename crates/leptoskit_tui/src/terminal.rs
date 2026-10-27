//! The real terminal: raw mode, the alternate screen, and crossterm's event
//! stream.
//!
//! The escape sequences and event translations live in pure functions so
//! tests can check them byte for byte; only the thin glue below touches the
//! process's own terminal.

use std::io;
use std::io::Write;
use std::time::Duration;

use crossterm::event::Event;
use crossterm::event::KeyEventKind;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
use crossterm::execute;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;

use crate::TuiError;
use crate::buffer::Buffer;
use crate::color::ColorDepth;
use crate::diff::diff;
use crate::host::OutputSink;
use crate::host::SourceEvent;
use crate::host::TuiHost;
use crate::host::TuiOptions;
use crate::input::Key;
use crate::input::KeyCode;
use crate::input::Mouse;

/// The frames already on screen, so each draw writes only the difference.
///
/// Pure: the terminal guard below owns one and points it at stdout.
#[derive(Debug, Default)]
pub struct FrameWriter {
	previous: Option<Buffer>,
	depth: ColorDepth,
}

impl FrameWriter {
	/// A writer for a terminal depth, starting from a blank screen.
	#[must_use]
	pub const fn new(depth: ColorDepth) -> Self {
		Self {
			previous: None,
			depth,
		}
	}

	/// Writes one frame's difference and records it as on screen.
	///
	/// # Errors
	///
	/// Returns the writer's errors unchanged.
	pub fn draw(&mut self, next: &Buffer, out: &mut impl Write) -> io::Result<bool> {
		// A missing previous frame is an all-blank buffer, so the first draw
		// repaints everything.
		let previous = self
			.previous
			.take()
			.unwrap_or_else(|| Buffer::new(next.width(), next.height()));
		let changed = diff(&previous, next, self.depth, out)?;
		self.previous = Some(next.clone());
		Ok(changed)
	}
}

/// Raw mode, the alternate screen, mouse capture, and a hidden cursor, all
/// restored on drop.
pub struct Terminal {
	frames: FrameWriter,
	mouse: bool,
	alternate: bool,
}

impl Terminal {
	/// Prepares the process's terminal.
	///
	/// # Errors
	///
	/// Returns an error when raw mode cannot be enabled or the setup
	/// sequences cannot be written; the terminal is unchanged then.
	pub fn new(options: &TuiOptions) -> Result<Self, TuiError> {
		crossterm::terminal::enable_raw_mode()?;

		let setup = setup_sequence(options);
		let mut out = io::stdout().lock();
		out.write_all(&setup)?;
		out.flush()?;

		Ok(Self {
			frames: FrameWriter::new(ColorDepth::detect()),
			mouse: options.mouse,
			alternate: options.alternate_screen,
		})
	}
}

/// The escape sequence that enters the terminal mode the options describe.
#[must_use]
pub fn setup_sequence(options: &TuiOptions) -> Vec<u8> {
	let mut setup = Vec::new();

	if options.alternate_screen {
		let _ = execute!(setup, EnterAlternateScreen);
	}

	if options.mouse {
		let _ = execute!(setup, crossterm::event::EnableMouseCapture);
	}

	let _ = execute!(setup, crossterm::cursor::Hide);

	if !options.title.is_empty() {
		let _ = execute!(setup, crossterm::terminal::SetTitle(&options.title));
	}

	setup
}

/// The escape sequence that undoes [`setup_sequence`].
#[must_use]
pub fn restore_sequence(mouse: bool, alternate: bool) -> Vec<u8> {
	let mut restore = Vec::new();

	if mouse {
		let _ = execute!(restore, crossterm::event::DisableMouseCapture);
	}

	let _ = execute!(restore, crossterm::cursor::Show);

	if alternate {
		let _ = execute!(restore, LeaveAlternateScreen);
	}

	restore
}

impl OutputSink for Terminal {
	fn size(&mut self) -> Result<(u16, u16), TuiError> {
		Ok(crossterm::terminal::size()?)
	}

	fn draw(&mut self, next: &Buffer) -> Result<bool, TuiError> {
		let mut out = io::BufWriter::new(io::stdout().lock());
		Ok(self.frames.draw(next, &mut out)?)
	}
}

impl Drop for Terminal {
	fn drop(&mut self) {
		let restore = restore_sequence(self.mouse, self.alternate);

		{
			let mut out = io::stdout().lock();
			let _ = out.write_all(&restore);
			let _ = out.flush();
		}

		let _ = crossterm::terminal::disable_raw_mode();
	}
}

/// Reads crossterm's event stream as host events.
#[derive(Default)]
pub struct CrosstermSource;

impl crate::host::EventSource for CrosstermSource {
	fn poll(&mut self, timeout: Duration) -> Result<bool, TuiError> {
		Ok(crossterm::event::poll(timeout)?)
	}

	fn read(&mut self) -> Result<Option<SourceEvent>, TuiError> {
		loop {
			match crossterm::event::read()? {
				Event::Key(key) => {
					if matches!(key.kind, KeyEventKind::Release) {
						continue;
					}

					return Ok(translate_event(Event::Key(key)));
				}
				event => return Ok(translate_event(event)),
			}
		}
	}
}

/// Translates one crossterm event, dropping the ones the backend does not
/// route.
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn translate_event(event: Event) -> Option<SourceEvent> {
	match event {
		Event::Key(key) => Some(SourceEvent::Key(key.into())),
		Event::Mouse(mouse) => {
			translate_mouse(mouse.kind, mouse.column, mouse.row).map(SourceEvent::Mouse)
		}
		Event::Resize(cols, rows) => Some(SourceEvent::Resize(cols, rows)),
		_ => None,
	}
}

/// Translates one crossterm mouse event; non-primary buttons and drags are
/// not routed.
#[must_use]
pub fn translate_mouse(kind: MouseEventKind, col: u16, row: u16) -> Option<Mouse> {
	match kind {
		MouseEventKind::Down(MouseButton::Left) => Some(Mouse::Click { col, row }),
		MouseEventKind::ScrollUp => Some(Mouse::ScrollUp { col, row }),
		MouseEventKind::ScrollDown => Some(Mouse::ScrollDown { col, row }),
		_ => None,
	}
}

impl From<crossterm::event::KeyEvent> for Key {
	fn from(event: crossterm::event::KeyEvent) -> Self {
		let code = match event.code {
			crossterm::event::KeyCode::Enter => KeyCode::Enter,
			crossterm::event::KeyCode::Char(ch) => KeyCode::Char(ch),
			crossterm::event::KeyCode::Tab => {
				if event
					.modifiers
					.contains(crossterm::event::KeyModifiers::SHIFT)
				{
					KeyCode::BackTab
				} else {
					KeyCode::Tab
				}
			}
			crossterm::event::KeyCode::Backspace => KeyCode::Backspace,
			crossterm::event::KeyCode::Delete => KeyCode::Delete,
			crossterm::event::KeyCode::Left => KeyCode::Left,
			crossterm::event::KeyCode::Right => KeyCode::Right,
			crossterm::event::KeyCode::Up => KeyCode::Up,
			crossterm::event::KeyCode::Down => KeyCode::Down,
			crossterm::event::KeyCode::Home => KeyCode::Home,
			crossterm::event::KeyCode::End => KeyCode::End,
			crossterm::event::KeyCode::PageUp => KeyCode::PageUp,
			crossterm::event::KeyCode::PageDown => KeyCode::PageDown,
			crossterm::event::KeyCode::Esc => KeyCode::Escape,
			_ => KeyCode::Other,
		};

		Self {
			code,
			ctrl: event
				.modifiers
				.contains(crossterm::event::KeyModifiers::CONTROL),
			alt: event
				.modifiers
				.contains(crossterm::event::KeyModifiers::ALT),
		}
	}
}

/// Runs a screen closure in the process's terminal; see
/// [`TuiHost`](crate::TuiHost) for the loop it runs.
///
/// # Errors
///
/// Returns an error when the terminal cannot be prepared, read, or written.
pub fn run(
	options: TuiOptions,
	view: impl FnMut() -> leptoskit_native::Element + 'static,
) -> Result<crate::host::Exit, TuiError> {
	let mut terminal = Terminal::new(&options)?;
	let mut source = CrosstermSource;
	let mut surface = leptoskit_native::NativeSurface::new(crate::TuiBackend::new());
	let mut app = crate::testing::surface_app(&mut surface, view);

	TuiHost::new(options).run(&mut *app, &mut source, &mut terminal)
}
