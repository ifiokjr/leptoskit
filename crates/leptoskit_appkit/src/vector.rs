//! Vector drawings as one `CAShapeLayer` per path inside a flipped view.

use leptoskit_native::Drawing;
use leptoskit_native::FillRule;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::Stroke;
use leptoskit_native::vector::PathCommand;
use leptoskit_native::vector::Transform;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2_app_kit::NSResponder;
use objc2_app_kit::NSView;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::NSArray;
use objc2_foundation::NSNumber;
use objc2_foundation::NSRect;
use objc2_quartz_core::CALayer;
use objc2_quartz_core::CAShapeLayer;
use objc2_quartz_core::CAShapeLayerFillRule;
use objc2_quartz_core::CATransaction;
use objc2_quartz_core::kCAFillRuleEvenOdd;
use objc2_quartz_core::kCAFillRuleNonZero;
use objc2_quartz_core::kCALineCapButt;
use objc2_quartz_core::kCALineCapRound;
use objc2_quartz_core::kCALineCapSquare;
use objc2_quartz_core::kCALineJoinBevel;
use objc2_quartz_core::kCALineJoinMiter;
use objc2_quartz_core::kCALineJoinRound;

use crate::text::ns_color;

define_class!(
	// SAFETY:
	// - `NSView` has no subclassing requirements beyond being initialized
	//   through its designated initializer, which `VectorView::new` does.
	// - The overridden methods match `NSView`'s signatures, and the backing
	//   change forwards to `super` first.
	// - The class does not implement `Drop`.
	#[unsafe(super(NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	/// A flipped view whose shape sublayers follow its window's pixel
	/// density, which AppKit only updates on the layers it owns.
	pub(crate) struct VectorView;

	impl VectorView {
		#[unsafe(method(isFlipped))]
		fn is_flipped(&self) -> bool {
			true
		}

		#[unsafe(method(viewDidChangeBackingProperties))]
		fn view_did_change_backing_properties(&self) {
			// SAFETY: `NSView` implements this notification method, which
			// takes no arguments and returns nothing.
			let () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
			let Some(layer) = self.layer() else {
				return;
			};
			let scale = backing_scale(self, &layer);
			// SAFETY: the view's layer holds only `CALayer`s.
			for shape in unsafe { layer.sublayers() }.iter().flat_map(|layers| layers.to_vec()) {
				shape.setContentsScale(scale);
			}
		}
	}
);

impl VectorView {
	/// A layer-backed flipped view with an empty frame.
	pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(());
		// SAFETY: `initWithFrame:` is `NSView`'s designated initializer and
		// returns an initialized instance of the receiver's class.
		let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
		view.setWantsLayer(true);
		view
	}
}

/// The pixels per point shape layers rasterize at: the window's, once the
/// view is in one.
fn backing_scale(view: &NSView, layer: &CALayer) -> f64 {
	view.window().map_or_else(
		|| layer.contentsScale(),
		|window| window.backingScaleFactor(),
	)
}

/// The shape layers drawing one `Vector`, one per path in painting order,
/// and the drawing they show.
///
/// Points are transformed before each path is built, so fixed strokes stay
/// in points however the drawing is scaled. The layers share the view's
/// flipped geometry and are not clipped to it unless the view clips.
pub(crate) struct VectorLayers {
	view: Retained<VectorView>,
	drawing: Option<Drawing>,
	layers: Vec<Retained<CAShapeLayer>>,
}

impl VectorLayers {
	/// No layers yet, drawing into `view`.
	pub(crate) fn new(view: Retained<VectorView>) -> Self {
		Self {
			view,
			drawing: None,
			layers: Vec::new(),
		}
	}

	/// Shows `drawing`, leaving the layers untouched when it is the drawing
	/// already shown.
	pub(crate) fn set_drawing(&mut self, drawing: Option<&Drawing>) {
		if self.drawing.as_ref() == drawing {
			return;
		}

		self.drawing = drawing.cloned();
		self.redraw();
	}

	/// Maps the drawing onto the view's current bounds, reusing the shape
	/// layers it already has and adding or removing only the difference.
	pub(crate) fn redraw(&mut self) {
		let Some(host) = self.view.layer() else {
			return;
		};
		let paths = self.drawing.as_ref().map_or(&[][..], Drawing::paths);
		let bounds = self.view.bounds();
		let scale = backing_scale(&self.view, &host);

		// Shape layers that do not back a view animate every property change
		// by default; a redraw must land in the same frame as its layout.
		CATransaction::begin();
		CATransaction::setDisableActions(true);
		for stale in self.layers.drain(paths.len().min(self.layers.len())..) {
			stale.removeFromSuperlayer();
		}
		while self.layers.len() < paths.len() {
			let shape = CAShapeLayer::new();
			host.addSublayer(&shape);
			self.layers.push(shape);
		}
		if let Some(drawing) = &self.drawing {
			let transform = drawing.transform(bounds.size.width as f32, bounds.size.height as f32);
			for (shape, path) in self.layers.iter().zip(paths) {
				shape.setFrame(bounds);
				shape.setContentsScale(scale);
				shape.setPath(Some(&cg_path(path.data().transformed(transform))));
				shape.setFillRule(fill_rule(path.rule()));
				let fill = path.fill_color().map(|color| ns_color(color).CGColor());
				shape.setFillColor(fill.as_deref());
				apply_stroke(shape, path.stroke_style(), transform);
			}
		}
		CATransaction::commit();
	}
}

fn fill_rule(rule: FillRule) -> &'static CAShapeLayerFillRule {
	// SAFETY: QuartzCore exports the fill-rule names as immutable static
	// strings.
	unsafe {
		match rule {
			FillRule::Nonzero => kCAFillRuleNonZero,
			FillRule::Evenodd => kCAFillRuleEvenOdd,
		}
	}
}

/// The stroke's color, width, dashes, and line style in frame units, or no
/// stroke at all. The miter limit is set explicitly, since Core
/// Animation's default is 10 and SVG's is 4.
fn apply_stroke(shape: &CAShapeLayer, stroke: Option<&Stroke>, transform: Transform) {
	let Some(stroke) = stroke else {
		shape.setStrokeColor(None);
		shape.setLineWidth(0.0);
		shape.setLineDashPattern(None);
		return;
	};
	shape.setStrokeColor(Some(&ns_color(stroke.color()).CGColor()));
	shape.setLineWidth(f64::from(stroke.frame_width(transform)));
	shape.setMiterLimit(f64::from(stroke.miter()));
	let dashes = stroke
		.frame_dashes(transform)
		.map(|length| NSNumber::new_f64(f64::from(length)))
		.collect::<Vec<_>>();
	let pattern = (!dashes.is_empty()).then(|| NSArray::from_retained_slice(&dashes));
	shape.setLineDashPattern(pattern.as_deref());
	shape.setLineDashPhase(f64::from(stroke.frame_dash_offset(transform)));
	// SAFETY: QuartzCore exports the cap and join names as immutable static
	// strings.
	unsafe {
		shape.setLineCap(match stroke.line_cap() {
			LineCap::Butt => kCALineCapButt,
			LineCap::Round => kCALineCapRound,
			LineCap::Square => kCALineCapSquare,
		});
		shape.setLineJoin(match stroke.line_join() {
			LineJoin::Miter => kCALineJoinMiter,
			LineJoin::Round => kCALineJoinRound,
			LineJoin::Bevel => kCALineJoinBevel,
		});
	}
}

/// Replays absolute commands, already in view coordinates, into a path.
fn cg_path(commands: impl Iterator<Item = PathCommand>) -> CFRetained<CGMutablePath> {
	let path = CGMutablePath::new();
	let target = Some(&*path);
	let identity = std::ptr::null();

	for command in commands {
		// SAFETY: a null transform means identity, and every coordinate was
		// validated finite when the path data was built.
		unsafe {
			match command {
				PathCommand::MoveTo(point) => {
					CGMutablePath::move_to_point(target, identity, point.x.into(), point.y.into());
				}
				PathCommand::LineTo(point) => {
					CGMutablePath::add_line_to_point(
						target,
						identity,
						point.x.into(),
						point.y.into(),
					);
				}
				PathCommand::QuadTo { control, to } => {
					CGMutablePath::add_quad_curve_to_point(
						target,
						identity,
						control.x.into(),
						control.y.into(),
						to.x.into(),
						to.y.into(),
					);
				}
				PathCommand::CubicTo {
					control1,
					control2,
					to,
				} => {
					CGMutablePath::add_curve_to_point(
						target,
						identity,
						control1.x.into(),
						control1.y.into(),
						control2.x.into(),
						control2.y.into(),
						to.x.into(),
						to.y.into(),
					);
				}
				PathCommand::Close => CGMutablePath::close_subpath(target),
			}
		}
	}

	path
}
