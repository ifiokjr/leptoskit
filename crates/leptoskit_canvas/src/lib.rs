//! <!-- {=canvasSummary|trim|linePrefix:"//! ":true} -->
//!
//! It is the drawing model for custom graphics that native widgets cannot
//! express: a `Scene` lists shapes, paths, gradients, shadows, clips, and
//! layers in order, and the wgpu renderer draws it on the GPU. It is standalone
//! for now; later work hosts it in native surfaces and the browser.
//!
//! <!-- {/canvasSummary} -->
//!
//! A [`Scene`] is plain data, so a Wasm guest can build one and send it to
//! its host. Hosts call [`Scene::sanitized`] (the [`Renderer`] does it for
//! them) to bound every number, list, and nesting depth before drawing.
//!
//! Rounded rectangles, borders, and box shadows are drawn as single quads
//! whose shader computes the shape's edge exactly, so they stay smooth at any
//! scale. Paths are cut into triangles with `lyon` and smoothed by 4×
//! multisampling, as are clip edges.
//!
//! ```no_run
//! use leptoskit_canvas::Color;
//! use leptoskit_canvas::FillRule;
//! use leptoskit_canvas::Path;
//! use leptoskit_canvas::Rect;
//! use leptoskit_canvas::Scene;
//!
//! let scene = Scene::new()
//! 	.fill_rounded_rect(
//! 		Rect::new(10.0, 10.0, 80.0, 40.0),
//! 		8.0,
//! 		Color::rgb8(37, 99, 235),
//! 	)
//! 	.fill_path(
//! 		Path::new().circle(50.0, 80.0, 12.0),
//! 		FillRule::NonZero,
//! 		Color::BLACK,
//! 	);
//! # #[cfg(all(feature = "renderer", not(target_arch = "wasm32")))]
//! let pixels = leptoskit_canvas::render_to_rgba(&scene, 200, 200, 2.0)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[cfg(feature = "renderer")]
mod encode;
mod model;
#[cfg(feature = "renderer")]
mod renderer;
mod sanitize;

pub use model::*;
#[cfg(feature = "renderer")]
pub use renderer::Error;
#[cfg(feature = "renderer")]
pub use renderer::Renderer;
#[cfg(all(feature = "renderer", not(target_arch = "wasm32")))]
pub use renderer::headless_device;
#[cfg(all(feature = "renderer", not(target_arch = "wasm32")))]
pub use renderer::render_to_rgba;
pub use sanitize::*;
/// The wgpu the renderer is built on, so hosts create devices and textures
/// with the same version.
#[cfg(feature = "renderer")]
pub use wgpu;
