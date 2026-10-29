//! Headless drivers for terminal app tests: a scripted event source and a
//! recording output sink.
//!
//! These are the same pieces [`terminal::run`](crate::terminal) wires to a
//! real terminal, so a test drives the whole application — focus, carets,
//! scrolling, repaints — against recorded frames instead of a tty.
//!
//! ```
//! use leptoskit_native::NativeSurface;
//! use leptoskit_native::components::Button;
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//! use leptoskit_tui::Key;
//! use leptoskit_tui::KeyCode;
//! use leptoskit_tui::SourceEvent;
//! use leptoskit_tui::TuiBackend;
//! use leptoskit_tui::TuiHost;
//! use leptoskit_tui::TuiOptions;
//! use leptoskit_tui::testing::Recorder;
//! use leptoskit_tui::testing::Scripted;
//!
//! let screen = || View().child(Text("Ready")).child(Button("Go"));
//! let mut surface = NativeSurface::new(TuiBackend::new());
//! let mut app = leptoskit_tui::testing::surface_app(&mut surface, screen);
//! let mut source = Scripted::new([SourceEvent::Key(Key::plain(KeyCode::Tab))]);
//! let mut sink = Recorder::new(40, 12);
//!
//! let exit = TuiHost::new(TuiOptions::default())
//! 	.run(&mut *app, &mut source, &mut sink)
//! 	.unwrap();
//! assert_eq!(exit, leptoskit_tui::Exit::SourceEnded);
//! assert!(sink.lines()[0].contains("Ready"));
//! ```

use std::collections::VecDeque;
use std::time::Duration;

use leptoskit_native::Element;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::NodeId;

use crate::TuiError;
use crate::backend::TuiBackend;
use crate::buffer::Buffer;
use crate::host::EventSource;
use crate::host::OutputSink;
use crate::host::SourceEvent;
use crate::host::TuiApplication;

/// An event source that replays queued events and then ends the session.
#[derive(Debug, Default)]
pub struct Scripted {
	events: VecDeque<SourceEvent>,
}

impl Scripted {
	/// A source replaying `events` in order.
	pub fn new(events: impl IntoIterator<Item = SourceEvent>) -> Self {
		Self {
			events: events.into_iter().collect(),
		}
	}
}

impl EventSource for Scripted {
	fn poll(&mut self, _timeout: Duration) -> Result<bool, TuiError> {
		Ok(!self.events.is_empty())
	}

	fn read(&mut self) -> Result<Option<SourceEvent>, TuiError> {
		Ok(self.events.pop_front())
	}

	fn done(&mut self) -> bool {
		self.events.is_empty()
	}
}

/// An output sink that records painted frames at a fixed size.
#[derive(Debug)]
pub struct Recorder {
	size: (u16, u16),
	/// Every frame drawn, oldest first.
	pub frames: Vec<Buffer>,
}

impl Recorder {
	/// A sink for a terminal of `cols` by `rows`.
	pub const fn new(cols: u16, rows: u16) -> Self {
		Self {
			size: (cols, rows),
			frames: Vec::new(),
		}
	}

	/// The last frame's rows as text; empty before the first draw.
	#[must_use]
	pub fn lines(&self) -> Vec<String> {
		self.frames
			.last()
			.map_or_else(Vec::new, Buffer::to_plain_lines)
	}
}

impl OutputSink for Recorder {
	fn size(&mut self) -> Result<(u16, u16), TuiError> {
		Ok(self.size)
	}

	fn draw(&mut self, next: &Buffer) -> Result<bool, TuiError> {
		let changed = self.frames.last().is_none_or(|previous| previous != next);

		self.frames.push(next.clone());
		Ok(changed)
	}
}

/// Wraps a surface and a screen closure as a [`TuiApplication`], re-rendering
/// after every dispatched event.
pub struct SurfaceApp<'a, F> {
	surface: &'a mut NativeSurface<TuiBackend>,
	view: F,
}

impl<F> TuiApplication for SurfaceApp<'_, F>
where
	F: FnMut() -> Element,
{
	fn backend(&mut self) -> &mut TuiBackend {
		self.surface.backend_mut()
	}

	fn dispatch(&mut self, node: NodeId, event: &NativeEvent) -> leptoskit_native::EventOutcome {
		let outcome = self.surface.dispatch_event(node, event);
		self.surface.set_root(&(self.view)());
		outcome
	}

	fn resize(&mut self, cols: u16, rows: u16) {
		let (width, height) = crate::viewport(cols, rows);
		self.surface.set_viewport(width, height);
	}

	fn mount(&mut self) {
		self.surface.set_root(&(self.view)());
	}
}

/// Wraps a surface and its screen closure as a host application.
pub fn surface_app<F>(surface: &mut NativeSurface<TuiBackend>, view: F) -> Box<SurfaceApp<'_, F>>
where
	F: FnMut() -> Element,
{
	Box::new(SurfaceApp { surface, view })
}
