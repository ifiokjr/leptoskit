//! AppKit adapters for an explicitly supplied, main-thread-owned window.

use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_app_kit::NSWindow;
use objc2_app_kit::NSWindowStyleMask;
use objc2_foundation::NSNumber;
use objc2_foundation::NSPoint;
use objc2_foundation::NSRect;
use objc2_foundation::NSSize;
use objc2_foundation::NSString;

use crate::Bounds;
use crate::Display;
use crate::Error;
use crate::WindowMode;
use crate::WindowState;

fn bounds(frame: NSRect) -> Result<Bounds, Error> {
	Bounds::new(
		frame.origin.x,
		frame.origin.y,
		frame.size.width,
		frame.size.height,
	)
}

fn display_id(screen: &NSScreen) -> Result<String, Error> {
	screen
		.deviceDescription()
		.objectForKey(&NSString::from_str("NSScreenNumber"))
		.and_then(|value| {
			value
				.downcast_ref::<NSNumber>()
				.map(|value| value.unsignedIntValue().to_string())
		})
		.ok_or(Error::InvalidDisplay)
}

/// Enumerates AppKit's visible work areas in logical points.
pub fn displays(main_thread: MainThreadMarker) -> Result<Vec<Display>, Error> {
	NSScreen::screens(main_thread)
		.iter()
		.map(|screen| Display::new(display_id(&screen)?, bounds(screen.visibleFrame())?))
		.collect()
}

/// Capture a particular application-owned window.
///
/// While zoomed, pass the bounds previously captured in normal mode. Fullscreen
/// is deliberately rejected: entering/leaving fullscreen is asynchronous and
/// requires the owning application's window-delegate lifecycle.
pub fn capture(window: &NSWindow, last_normal: Option<Bounds>) -> Result<WindowState, Error> {
	if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
		return Err(Error::Unavailable);
	}
	let mode = if window.isZoomed() {
		WindowMode::Maximized
	} else {
		WindowMode::Normal
	};
	let normal_bounds = if mode == WindowMode::Maximized {
		last_normal.ok_or(Error::MissingNormalBounds)?
	} else {
		bounds(window.frame())?
	};

	Ok(WindowState {
		normal_bounds,
		display_id: window.screen().as_deref().map(display_id).transpose()?,
		mode,
		minimized: window.isMiniaturized(),
	})
}

/// Restore onto currently connected work areas without activating the window.
///
/// The host chooses when this is appropriate (normally before showing the
/// window). This function never searches for or changes another window.
pub fn restore(
	window: &NSWindow,
	state: &WindowState,
	main_thread: MainThreadMarker,
) -> Result<(), Error> {
	if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
		return Err(Error::Unavailable);
	}

	let placed = crate::placement(state, &displays(main_thread)?)?;

	if window.isMiniaturized() {
		window.deminiaturize(None);
	}

	if window.isZoomed() {
		window.zoom(None);
	}

	let frame = placed.bounds;
	window.setFrame_display(
		NSRect::new(
			NSPoint::new(frame.x(), frame.y()),
			NSSize::new(frame.width(), frame.height()),
		),
		false,
	);

	if placed.mode == WindowMode::Maximized {
		window.zoom(None);
	}

	if placed.minimized {
		window.miniaturize(None);
	}

	Ok(())
}
