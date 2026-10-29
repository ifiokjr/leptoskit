//! The terminal backend: the same shared tree, drawn in a cell grid.
//!
//! Apps describe their interface once, as [`Element`] trees with Tailwind
//! classes, and every platform commits that tree through a
//! [`NativeSurface`](leptoskit_native::NativeSurface) into its own backend.
//! This crate's backend is the terminal: layout lands on the monospace grid,
//! keyboard focus, carets, and scrolling live in the backend the way GTK
//! owns its own input, and events surface through the shared
//! [`NativeEvent`](leptoskit_native::NativeEvent) model.
//!
//! A terminal app is for a long-lived interface — an editor, a dashboard, a
//! chat client — the way Ink is for React. A short-lived command-line tool
//! still belongs to an argument parser.
//!
//! # Embedding
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_native::NativeSurface;
//! use leptoskit_tui::TuiBackend;
//!
//! let mut surface = NativeSurface::new(TuiBackend::new());
//! surface.set_viewport(80.0 * 8.0, 24.0 * 16.0);
//! surface.set_root(&Element::text("Hello, terminal"));
//! let frame = surface.backend().render(80, 24);
//! assert_eq!(frame.to_plain_lines()[0].trim_end(), "Hello, terminal");
//! ```
//!
//! # Running
//!
//! On a process with a real terminal, [`terminal::run`] owns raw mode, the
//! alternate screen, the event loop, and diff-based repaints:
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::sync::atomic::AtomicUsize;
//!
//! use leptoskit_native::components::Button;
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//! use leptoskit_tui::TuiOptions;
//!
//! let presses = Arc::new(AtomicUsize::new(0));
//! let screen = {
//! 	let presses = Arc::clone(&presses);
//! 	move || {
//! 		View()
//! 			.child(Text(format!(
//! 				"Pressed {} times",
//! 				presses.load(std::sync::atomic::Ordering::Relaxed)
//! 			)))
//! 			.child(Button("Press").on_press({
//! 				let presses = Arc::clone(&presses);
//! 				move || {
//! 					presses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
//! 				}
//! 			}))
//! 	}
//! };
//!
//! leptoskit_tui::terminal::run(TuiOptions::default(), screen).unwrap();
//! ```
//!
//! The cell is the unit: one cell is 8×16 logical pixels, so Tailwind spacing
//! (`p-4` is sixteen pixels) and the grid agree, and the terminal's font is
//! the user's own.

pub mod backend;
pub mod buffer;
pub mod cell;
pub mod color;
pub mod diff;
pub mod focus;
pub mod host;
pub mod input;
pub mod paint;
pub mod testing;
pub mod text;

#[cfg(not(target_arch = "wasm32"))]
pub mod terminal;

pub use backend::Draft;
pub use backend::Frame;
pub use backend::Node;
pub use backend::Scroll;
pub use backend::TuiBackend;
pub use backend::viewport;
pub use buffer::Buffer;
pub use buffer::Rect;
pub use cell::Attributes;
pub use cell::Cell;
pub use cell::TuiColor;
pub use color::ColorDepth;
pub use host::EventSource;
pub use host::Exit;
pub use host::OutputSink;
pub use host::SourceEvent;
pub use host::TuiApplication;
pub use host::TuiHost;
pub use host::TuiOptions;
pub use input::Dispatch;
pub use input::Key;
pub use input::KeyCode;
pub use input::Mouse;
pub use text::TuiTextMeasurer;
pub use text::display_width;
pub use text::wrap;
pub use text::wrap_ranges;

/// A terminal input/output failure.
#[derive(Debug, thiserror::Error)]
#[error("terminal error: {0}")]
pub struct TuiError(#[from] std::io::Error);
