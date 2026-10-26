//! Vector drawings, painted anti-aliased through the GDI+ flat API.
//!
//! GDI cannot anti-alias paths, so drawings go through GDI+ onto the same
//! device contexts the rest of the backend paints with GDI. Each path is
//! mapped through the drawing's transform and the DPI scale before it reaches
//! GDI+, so a stroke's width stays in layout units however the view box
//! scales. Fills use the nonzero winding rule, and miter joins bevel past
//! [`Stroke::MITER_LIMIT`], as in SVG.

use std::ptr::null;
use std::ptr::null_mut;
use std::sync::OnceLock;

use leptoskit_native::Color;
use leptoskit_native::Drawing;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::Stroke;
use leptoskit_native::VectorPath;
use leptoskit_native::vector::PathCommand;
use leptoskit_native::vector::Point;
use leptoskit_native::vector::Transform;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::CreateCompatibleDC;
use windows::Win32::Graphics::Gdi::DeleteDC;
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::Graphics::Gdi::GetRgnBox;
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Gdi::HRGN;
use windows::Win32::Graphics::Gdi::NULLREGION;
use windows::Win32::Graphics::GdiPlus as gdiplus;

/// Device pixels added around painted geometry when computing its region,
/// so the anti-aliased fringe of an edge is never cut off.
const FRINGE: f32 = 1.0;

static STARTUP: OnceLock<usize> = OnceLock::new();

/// Starts GDI+ for the process, once.
///
/// GDI+ stays started for the life of the process. Its objects only live
/// inside a single paint or region computation, but backends on several UI
/// threads share the one process-wide startup, so no single backend may shut
/// it down.
pub(crate) fn start() -> windows::core::Result<()> {
	if STARTUP.get().is_some() {
		return Ok(());
	}

	let input = gdiplus::GdiplusStartupInput {
		GdiplusVersion: 1,
		..Default::default()
	};
	let mut token = 0;
	// SAFETY: The input outlives the call. A null output is permitted while
	// GDI+ runs its own background thread.
	let status = unsafe { gdiplus::GdiplusStartup(&raw mut token, &raw const input, null_mut()) };

	if status != gdiplus::Ok {
		return Err(windows::core::Error::new(
			E_FAIL,
			format!("GDI+ could not start (status {})", status.0),
		));
	}

	if STARTUP.set(token).is_err() {
		// Another thread started GDI+ first. Startups are reference counted,
		// so release this one.
		// SAFETY: The token came from the successful startup above.
		unsafe { gdiplus::GdiplusShutdown(token) };
	}

	Ok(())
}

/// Asserts that a GDI+ allocation or path-building call succeeded.
fn check(status: gdiplus::Status, operation: &str) {
	assert_eq!(status, gdiplus::Ok, "GDI+ could not {operation}");
}

/// The GDI+ ARGB form of a packed RGBA color.
fn argb(color: Color) -> u32 {
	(u32::from(color.alpha()) << 24)
		| (u32::from(color.red()) << 16)
		| (u32::from(color.green()) << 8)
		| u32::from(color.blue())
}

/// An owned GDI region.
pub(crate) struct Region(pub HRGN);

impl Region {
	/// The bounding box, or `None` for an empty region.
	pub fn bounds(&self) -> Option<RECT> {
		let mut rect = RECT::default();
		// SAFETY: The region is owned and live; GDI writes one rectangle.
		let kind = unsafe { GetRgnBox(self.0, &raw mut rect) };
		(kind != NULLREGION).then_some(rect)
	}
}

impl Drop for Region {
	fn drop(&mut self) {
		// SAFETY: This uniquely owned region was never given to a window.
		unsafe {
			let _ = DeleteObject(self.0.into());
		}
	}
}

struct Graphics(*mut gdiplus::GpGraphics);

impl Graphics {
	/// A GDI+ surface over `dc`, measuring in device pixels.
	fn new(dc: HDC) -> Self {
		let mut graphics = null_mut();
		// SAFETY: The caller keeps `dc` alive and unused by GDI until this
		// surface is dropped.
		unsafe {
			check(
				gdiplus::GdipCreateFromHDC(dc, &raw mut graphics),
				"wrap a device context",
			);
			let graphics = Self(graphics);
			check(
				gdiplus::GdipSetPageUnit(graphics.0, gdiplus::UnitPixel),
				"measure in pixels",
			);
			graphics
		}
	}

	/// A surface that anti-aliases, with pixel centres at half-pixel
	/// coordinates as in SVG, so a one-unit line on a whole coordinate covers
	/// exactly one row of pixels.
	fn antialiased(dc: HDC) -> Self {
		let graphics = Self::new(dc);
		// SAFETY: The surface is live. 8x8 sampling needs GDI+ 1.1; older
		// runtimes reject it, and 8x4 sampling is the best they offer.
		unsafe {
			if gdiplus::GdipSetSmoothingMode(graphics.0, gdiplus::SmoothingModeAntiAlias8x8)
				!= gdiplus::Ok
			{
				check(
					gdiplus::GdipSetSmoothingMode(graphics.0, gdiplus::SmoothingModeAntiAlias),
					"anti-alias",
				);
			}
			check(
				gdiplus::GdipSetPixelOffsetMode(graphics.0, gdiplus::PixelOffsetModeHalf),
				"offset pixel centres",
			);
		}
		graphics
	}
}

impl Drop for Graphics {
	fn drop(&mut self) {
		// SAFETY: The surface is uniquely owned.
		unsafe {
			gdiplus::GdipDeleteGraphics(self.0);
		}
	}
}

struct Path(*mut gdiplus::GpPath);

impl Path {
	/// `path` in device pixels, offset by `origin`, or `None` when it has no
	/// segments.
	fn new(path: &VectorPath, transform: Transform, scale: f32, origin: [f32; 2]) -> Option<Self> {
		let device = |point: Point| {
			Point::new(
				point.x.mul_add(scale, origin[0]),
				point.y.mul_add(scale, origin[1]),
			)
		};
		// A quadratic Bézier is the cubic whose control points lie two thirds
		// of the way from each end point to the quadratic control point.
		let toward = |from: Point, control: Point| {
			Point::new(
				(control.x - from.x).mul_add(2.0 / 3.0, from.x),
				(control.y - from.y).mul_add(2.0 / 3.0, from.y),
			)
		};
		let mut geometry = null_mut();
		// SAFETY: GDI+ writes one owned path.
		unsafe {
			check(
				gdiplus::GdipCreatePath(gdiplus::FillModeWinding, &raw mut geometry),
				"allocate a path",
			);
		}
		let geometry = Self(geometry);
		let mut start = Point::default();
		let mut current = Point::default();
		// GDI+ closes the path's last figure on a close, so a close only
		// applies once the current subpath has a segment of its own.
		let mut drawn = false;

		for command in path.data().transformed(transform) {
			// SAFETY: Every call appends to the owned, live path.
			unsafe {
				match command {
					PathCommand::MoveTo(point) => {
						check(gdiplus::GdipStartPathFigure(geometry.0), "start a subpath");
						current = device(point);
						start = current;
						drawn = false;
					}
					PathCommand::LineTo(point) => {
						let to = device(point);
						check(
							gdiplus::GdipAddPathLine(geometry.0, current.x, current.y, to.x, to.y),
							"add a line",
						);
						current = to;
						drawn = true;
					}
					PathCommand::QuadTo { control, to } => {
						let [control, to] = [device(control), device(to)];
						let [first, second] = [toward(current, control), toward(to, control)];
						check(
							gdiplus::GdipAddPathBezier(
								geometry.0, current.x, current.y, first.x, first.y, second.x,
								second.y, to.x, to.y,
							),
							"add a curve",
						);
						current = to;
						drawn = true;
					}
					PathCommand::CubicTo {
						control1,
						control2,
						to,
					} => {
						let [first, second, to] = [device(control1), device(control2), device(to)];
						check(
							gdiplus::GdipAddPathBezier(
								geometry.0, current.x, current.y, first.x, first.y, second.x,
								second.y, to.x, to.y,
							),
							"add a curve",
						);
						current = to;
						drawn = true;
					}
					PathCommand::Close => {
						if drawn {
							check(gdiplus::GdipClosePathFigure(geometry.0), "close a subpath");
						}
						current = start;
					}
				}
			}
		}

		let mut count = 0;
		// SAFETY: The path is live; GDI+ writes one count.
		unsafe {
			check(
				gdiplus::GdipGetPointCount(geometry.0, &raw mut count),
				"count path points",
			);
		}
		(count > 0).then_some(geometry)
	}
}

impl Drop for Path {
	fn drop(&mut self) {
		// SAFETY: The path is uniquely owned.
		unsafe {
			gdiplus::GdipDeletePath(self.0);
		}
	}
}

struct Pen(*mut gdiplus::GpPen);

impl Pen {
	/// A pen shaped like `stroke`, `width` device pixels wide.
	fn new(stroke: Stroke, width: f32) -> Self {
		let cap = match stroke.line_cap() {
			LineCap::Butt => gdiplus::LineCapFlat,
			LineCap::Round => gdiplus::LineCapRound,
			LineCap::Square => gdiplus::LineCapSquare,
		};
		// GDI+'s plain miter join clips long miters at the limit; SVG bevels
		// them, which is GDI+'s clipped-miter join.
		let join = match stroke.line_join() {
			LineJoin::Miter => gdiplus::LineJoinMiterClipped,
			LineJoin::Round => gdiplus::LineJoinRound,
			LineJoin::Bevel => gdiplus::LineJoinBevel,
		};
		let mut pen = null_mut();
		// SAFETY: GDI+ writes one owned pen, then configures it while live.
		unsafe {
			check(
				gdiplus::GdipCreatePen1(
					argb(stroke.color()),
					width,
					gdiplus::UnitWorld,
					&raw mut pen,
				),
				"allocate a pen",
			);
			let pen = Self(pen);
			check(
				gdiplus::GdipSetPenLineCap197819(pen.0, cap, cap, gdiplus::DashCapFlat),
				"set line caps",
			);
			check(gdiplus::GdipSetPenLineJoin(pen.0, join), "set line joins");
			check(
				gdiplus::GdipSetPenMiterLimit(pen.0, Stroke::MITER_LIMIT),
				"set the miter limit",
			);
			pen
		}
	}
}

impl Drop for Pen {
	fn drop(&mut self) {
		// SAFETY: The pen is uniquely owned.
		unsafe {
			gdiplus::GdipDeletePen(self.0);
		}
	}
}

struct Brush(*mut gdiplus::GpSolidFill);

impl Brush {
	fn new(color: Color) -> Self {
		let mut brush = null_mut();
		// SAFETY: GDI+ writes one owned brush.
		unsafe {
			check(
				gdiplus::GdipCreateSolidFill(argb(color), &raw mut brush),
				"allocate a brush",
			);
		}
		Self(brush)
	}
}

impl Drop for Brush {
	fn drop(&mut self) {
		// SAFETY: The brush is uniquely owned; a solid fill is a brush.
		unsafe {
			gdiplus::GdipDeleteBrush(self.0.cast());
		}
	}
}

struct PathRegion(*mut gdiplus::GpRegion);

impl PathRegion {
	fn empty() -> Self {
		let mut region = null_mut();
		// SAFETY: GDI+ writes one owned, initially infinite region.
		unsafe {
			check(
				gdiplus::GdipCreateRegion(&raw mut region),
				"allocate a region",
			);
			let region = Self(region);
			check(gdiplus::GdipSetEmpty(region.0), "empty a region");
			region
		}
	}

	fn add(&self, path: &Path) {
		// SAFETY: The region and path are live; GDI+ copies the geometry.
		unsafe {
			check(
				gdiplus::GdipCombineRegionPath(self.0, path.0, gdiplus::CombineModeUnion),
				"add a path to a region",
			);
		}
	}

	/// Adds the area `pen` covers when it strokes `path`.
	fn add_outline(&self, path: &Path, pen: &Pen) {
		let mut outline = null_mut();
		// SAFETY: The clone is owned; widening replaces its geometry in place.
		unsafe {
			check(
				gdiplus::GdipClonePath(path.0, &raw mut outline),
				"copy a path",
			);
			let outline = Path(outline);

			if gdiplus::GdipWidenPath(outline.0, pen.0, null_mut(), gdiplus::FlatnessDefault)
				== gdiplus::Ok
			{
				// A widened outline overlaps itself wherever the stroke does.
				check(
					gdiplus::GdipSetPathFillMode(outline.0, gdiplus::FillModeWinding),
					"set a fill mode",
				);
				self.add(&outline);
				return;
			}
		}

		// GDI+ cannot widen some degenerate geometry, such as a lone
		// zero-length segment; its pen-inflated bounds cover whatever it
		// paints.
		let mut bounds = gdiplus::RectF::default();
		// SAFETY: The path and pen are live; GDI+ writes one rectangle.
		unsafe {
			check(
				gdiplus::GdipGetPathWorldBounds(path.0, &raw mut bounds, null(), pen.0),
				"measure a path",
			);
			check(
				gdiplus::GdipCombineRegionRect(
					self.0,
					&raw const bounds,
					gdiplus::CombineModeUnion,
				),
				"add bounds to a region",
			);
		}
	}
}

impl Drop for PathRegion {
	fn drop(&mut self) {
		// SAFETY: The region is uniquely owned.
		unsafe {
			gdiplus::GdipDeleteRegion(self.0);
		}
	}
}

/// Paints `drawing` over a `width` by `height` frame, in layout units, whose
/// top-left corner sits at `origin` in `dc`'s device pixels. Paths paint in
/// order, each filled and then stroked.
pub(crate) fn paint(
	dc: HDC,
	drawing: &Drawing,
	[width, height]: [f32; 2],
	scale: f32,
	origin: [f32; 2],
) {
	let transform = drawing.transform(width, height);
	let graphics = Graphics::antialiased(dc);

	for path in drawing.paths() {
		let fill = path.fill_color();
		let stroke = path.stroke_style();

		if fill.is_none() && stroke.is_none() {
			continue;
		}
		let Some(geometry) = Path::new(path, transform, scale, origin) else {
			continue;
		};

		// GDI+ refuses to rasterize geometry outside its fixed-point range;
		// such a path stays unpainted rather than aborting the window's paint.
		if let Some(color) = fill {
			let brush = Brush::new(color);
			// SAFETY: The surface, brush, and path are live.
			let _ = unsafe { gdiplus::GdipFillPath(graphics.0, brush.0.cast(), geometry.0) };
		}

		if let Some(stroke) = stroke {
			let pen = Pen::new(stroke, stroke.width() * scale);
			// SAFETY: The surface, pen, and path are live.
			let _ = unsafe { gdiplus::GdipDrawPath(graphics.0, pen.0, geometry.0) };
		}
	}
}

/// The device pixels `drawing` paints over a `width` by `height` frame, in
/// layout units, relative to the frame's top-left pixel. The region includes
/// a pixel of anti-aliased fringe around every edge and ignores the frame's
/// bounds: strokes and curves may paint past it.
pub(crate) fn ink(drawing: &Drawing, [width, height]: [f32; 2], scale: f32) -> Region {
	let transform = drawing.transform(width, height);
	let region = PathRegion::empty();
	let fringe = Pen::new(
		Stroke::new(Color::rgba(0, 0, 0, 255), 1.0)
			.expect("a positive width is a valid stroke")
			.round(),
		FRINGE * 2.0,
	);

	for path in drawing.paths() {
		let Some(geometry) = Path::new(path, transform, scale, [0.0, 0.0]) else {
			continue;
		};

		if path.fill_color().is_some() {
			region.add(&geometry);
			region.add_outline(&geometry, &fringe);
		}

		if let Some(stroke) = path.stroke_style() {
			let pen = Pen::new(stroke, stroke.width().mul_add(scale, FRINGE * 2.0));
			region.add_outline(&geometry, &pen);
		}
	}

	// SAFETY: The memory DC only hosts the GDI+ surface that converts the
	// region; the surface is dropped before the DC is deleted.
	unsafe {
		let dc = CreateCompatibleDC(None);
		assert!(!dc.is_invalid(), "Windows could not allocate a region DC");
		let graphics = Graphics::new(dc);
		let mut hrgn = HRGN::default();
		check(
			gdiplus::GdipGetRegionHRgn(region.0, graphics.0, &raw mut hrgn),
			"convert a region",
		);
		drop(graphics);
		let _ = DeleteDC(dc);
		Region(hrgn)
	}
}
