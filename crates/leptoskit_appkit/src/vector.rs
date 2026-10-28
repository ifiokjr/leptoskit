//! Vector drawings as one `CAShapeLayer` per path inside a flipped view.

use leptoskit_native::Drawing;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::Stroke;
use leptoskit_native::vector::PathCommand;
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
use objc2_foundation::NSRect;
use objc2_quartz_core::CALayer;
use objc2_quartz_core::CAShapeLayer;
use objc2_quartz_core::CATransaction;
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

/// Replaces the view's sublayers with the drawing mapped onto its bounds.
///
/// Points are transformed before the path is built, so stroke widths stay
/// in points however the drawing is scaled. The layers share the view's
/// flipped geometry and are not clipped to it unless the view clips.
pub(crate) fn draw(view: &NSView, drawing: &Drawing) {
	let Some(layer) = view.layer() else {
		return;
	};
	let bounds = view.bounds();
	let transform = drawing.transform(bounds.size.width as f32, bounds.size.height as f32);
	let scale = backing_scale(view, &layer);
	let shapes = drawing
		.paths()
		.iter()
		.filter(|path| path.fill_color().is_some() || path.stroke_style().is_some())
		.map(|path| {
			let shape = CAShapeLayer::new();
			shape.setFrame(bounds);
			shape.setContentsScale(scale);
			shape.setPath(Some(&cg_path(path.data().transformed(transform))));
			// SAFETY: QuartzCore exports the fill-rule name as an immutable
			// static string.
			shape.setFillRule(unsafe { kCAFillRuleNonZero });
			let fill = path.fill_color().map(|color| ns_color(color).CGColor());
			shape.setFillColor(fill.as_deref());

			if let Some(stroke) = path.stroke_style() {
				apply_stroke(&shape, stroke);
			}

			Retained::into_super(shape)
		})
		.collect::<Vec<Retained<CALayer>>>();

	// New layers would otherwise fade in through the implicit sublayer
	// action, lagging behind the views around them.
	CATransaction::begin();
	CATransaction::setDisableActions(true);
	// SAFETY: every element is a `CALayer`, and the view draws nothing else
	// into its own sublayers.
	unsafe { layer.setSublayers(Some(&NSArray::from_retained_slice(&shapes))) };
	CATransaction::commit();
}

/// The stroke's width, color, and line style, with SVG's miter limit,
/// since Core Animation's default is 10.
fn apply_stroke(shape: &CAShapeLayer, stroke: Stroke) {
	shape.setStrokeColor(Some(&ns_color(stroke.color()).CGColor()));
	shape.setLineWidth(f64::from(stroke.width()));
	shape.setMiterLimit(f64::from(Stroke::MITER_LIMIT));
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
