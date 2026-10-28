//! Draws `Vector` paths as Core Animation shape layers.
//!
//! Each path owns one `CAShapeLayer`, stacked in painting order over the view's
//! background, so a path's fill sits under its own stroke and later paths
//! cover earlier ones. Points map onto the frame before the `CGPath` is built,
//! so stroke widths stay in layout units however the view box scales. Shape
//! layers never mask themselves; the view clips only when its props ask.

use leptoskit_native::Color;
use leptoskit_native::Drawing;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::PathData;
use leptoskit_native::Stroke;
use leptoskit_native::vector::PathCommand;
use leptoskit_native::vector::Transform;
use objc2::rc::Retained;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGColor;
use objc2_core_graphics::CGMutablePath;
use objc2_quartz_core::CAShapeLayer;
use objc2_quartz_core::CAShapeLayerLineCap;
use objc2_quartz_core::CAShapeLayerLineJoin;
use objc2_quartz_core::CATransaction;
use objc2_quartz_core::kCAFillRuleNonZero;
use objc2_quartz_core::kCALineCapButt;
use objc2_quartz_core::kCALineCapRound;
use objc2_quartz_core::kCALineCapSquare;
use objc2_quartz_core::kCALineJoinBevel;
use objc2_quartz_core::kCALineJoinMiter;
use objc2_quartz_core::kCALineJoinRound;
use objc2_ui_kit::UIView;

/// The shape layers drawing one `Vector`, one per path in painting order.
#[derive(Default)]
pub(crate) struct VectorLayers {
	layers: Vec<Retained<CAShapeLayer>>,
}

impl VectorLayers {
	/// Redraws `drawing` into the view's current bounds, reusing layers.
	pub(crate) fn draw(&mut self, view: &UIView, drawing: Option<&Drawing>) {
		let paths = drawing.map_or(&[][..], Drawing::paths);
		let bounds = view.bounds();
		// Layers that do not back a view animate every property change by
		// default; a redraw must land in the same frame as its layout.
		CATransaction::begin();
		CATransaction::setDisableActions(true);
		for stale in self.layers.drain(paths.len().min(self.layers.len())..) {
			stale.removeFromSuperlayer();
		}
		while self.layers.len() < paths.len() {
			let layer = CAShapeLayer::new();
			view.layer().addSublayer(&layer);
			self.layers.push(layer);
		}
		if let Some(drawing) = drawing {
			let transform = drawing.transform(bounds.size.width as f32, bounds.size.height as f32);
			for (layer, path) in self.layers.iter().zip(paths) {
				layer.setFrame(bounds);
				layer.setPath(Some(&cg_path(path.data(), transform)));
				// SAFETY: Core Animation exports this immutable constant.
				layer.setFillRule(unsafe { kCAFillRuleNonZero });
				layer.setFillColor(path.fill_color().map(cg_color).as_deref());
				apply_stroke(layer, path.stroke_style());
			}
		}
		CATransaction::commit();
	}
}

/// Builds a platform path from `data` mapped onto the frame by `transform`.
fn cg_path(data: &PathData, transform: Transform) -> CFRetained<CGMutablePath> {
	let path = CGMutablePath::new();
	let target = Some(&*path);
	let none = std::ptr::null();
	for command in data.transformed(transform) {
		// SAFETY: Every point is already in frame units, so each call passes
		// a null transform, which Core Graphics accepts.
		unsafe {
			match command {
				PathCommand::MoveTo(to) => {
					CGMutablePath::move_to_point(target, none, f64::from(to.x), f64::from(to.y));
				}
				PathCommand::LineTo(to) => {
					CGMutablePath::add_line_to_point(
						target,
						none,
						f64::from(to.x),
						f64::from(to.y),
					);
				}
				PathCommand::QuadTo { control, to } => {
					CGMutablePath::add_quad_curve_to_point(
						target,
						none,
						f64::from(control.x),
						f64::from(control.y),
						f64::from(to.x),
						f64::from(to.y),
					);
				}
				PathCommand::CubicTo {
					control1,
					control2,
					to,
				} => {
					CGMutablePath::add_curve_to_point(
						target,
						none,
						f64::from(control1.x),
						f64::from(control1.y),
						f64::from(control2.x),
						f64::from(control2.y),
						f64::from(to.x),
						f64::from(to.y),
					);
				}
				PathCommand::Close => CGMutablePath::close_subpath(target),
			}
		}
	}
	path
}

fn apply_stroke(layer: &CAShapeLayer, stroke: Option<Stroke>) {
	let Some(stroke) = stroke else {
		layer.setStrokeColor(None);
		layer.setLineWidth(0.0);
		return;
	};
	layer.setStrokeColor(Some(&cg_color(stroke.color())));
	layer.setLineWidth(f64::from(stroke.width()));
	layer.setMiterLimit(f64::from(Stroke::MITER_LIMIT));
	layer.setLineCap(line_cap(stroke.line_cap()));
	layer.setLineJoin(line_join(stroke.line_join()));
}

fn line_cap(cap: LineCap) -> &'static CAShapeLayerLineCap {
	// SAFETY: Core Animation exports these immutable constants.
	unsafe {
		match cap {
			LineCap::Butt => kCALineCapButt,
			LineCap::Round => kCALineCapRound,
			LineCap::Square => kCALineCapSquare,
		}
	}
}

fn line_join(join: LineJoin) -> &'static CAShapeLayerLineJoin {
	// SAFETY: Core Animation exports these immutable constants.
	unsafe {
		match join {
			LineJoin::Miter => kCALineJoinMiter,
			LineJoin::Round => kCALineJoinRound,
			LineJoin::Bevel => kCALineJoinBevel,
		}
	}
}

fn cg_color(color: Color) -> CFRetained<CGColor> {
	CGColor::new_srgb(
		f64::from(color.red()) / 255.0,
		f64::from(color.green()) / 255.0,
		f64::from(color.blue()) / 255.0,
		f64::from(color.alpha()) / 255.0,
	)
}
