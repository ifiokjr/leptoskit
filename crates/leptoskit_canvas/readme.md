<h1 align="center">leptoskit_canvas</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_canvas</strong> draws a 2D display list of rounded rectangles, shadows, gradients, and vector paths on the GPU with wgpu.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_canvas"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_canvas"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_canvas"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_canvas.svg"></a>
  <a href="https://docs.rs/leptoskit_canvas"><img alt="docs.rs" src="https://docs.rs/leptoskit_canvas/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Some interfaces need drawing that no native widget offers: a chart, a progress ring, a card with a soft shadow and a gradient. `leptoskit_canvas` describes that drawing as a `Scene`, an ordered list of commands in the style of React Native Skia, and renders it with wgpu on Metal, Vulkan, Direct3D 12, or WebGPU.

- **Shapes:** rectangles with a radius per corner and an optional border, and CSS-style box shadows (offset, blur, spread, colour).
- **Paints:** solid colours, linear gradients, and radial gradients, mixed with premultiplied alpha as CSS mixes them.
- **Paths:** lines, quadratic and cubic Bézier curves, filled with the non-zero or even-odd rule, or stroked with butt, round, or square caps and miter, round, or bevel joins.
- **Structure:** clips (rectangles, rounded rectangles, or paths, which intersect when nested), layers blended at an opacity, and affine transforms.

Colours are sRGB with straight alpha, channels from 0 to 1. The renderer premultiplies them and blends the way browsers do when it draws into a format without sRGB encoding.

## How it fits

<!-- {=canvasSummary} -->

It is the drawing model for custom graphics that native widgets cannot express: a `Scene` lists shapes, paths, gradients, shadows, clips, and layers in order, and the wgpu renderer draws it on the GPU. It is standalone for now; later work hosts it in native surfaces and the browser.

<!-- {/canvasSummary} -->

A scene is plain data (with the `serde` feature, it serializes), so a Wasm guest can build one and send it to its host. `Scene::sanitized` bounds every number, list, and nesting depth, and the renderer applies it to every scene it draws, so a scene from untrusted code cannot make the host allocate without limit.

Without the default `renderer` feature, the crate is only the display list and builds for any target, Wasm guests included.

## How it draws

Rounded rectangles, borders, and shadows are each one instanced quad. The shader computes the shape's signed distance per pixel, so edges stay smooth at any scale, and shadows use an analytic Gaussian blur instead of a blur pass. Paths are cut into triangles with [`lyon`](https://github.com/nical/lyon) and smoothed by 4× multisampling, as are clip edges, which are kept in a stencil buffer. Each open layer draws into a texture of its own that is blended into its parent when it closes.

The quad shader's rounded-rectangle distance and box shadow are adapted from [Rune](https://github.com/anomalyco/rune), under the MIT license in [`LICENSE-rune`](LICENSE-rune).

## Use it

```rust
use leptoskit_canvas::BoxShadow;
use leptoskit_canvas::Color;
use leptoskit_canvas::FillRule;
use leptoskit_canvas::Path;
use leptoskit_canvas::Rect;
use leptoskit_canvas::Scene;

let card = Rect::new(16.0, 16.0, 160.0, 96.0);
let scene = Scene::new()
    .box_shadow(card, 12.0, BoxShadow {
        offset_y: 4.0,
        blur: 16.0,
        color: Color::rgba(0.0, 0.0, 0.0, 0.25),
        ..BoxShadow::default()
    })
    .fill_rounded_rect(card, 12.0, Color::WHITE)
    .fill_path(Path::new().circle(56.0, 64.0, 24.0), FillRule::NonZero, Color::rgb8(37, 99, 235));

// Offscreen, on a device of its own: RGBA bytes with premultiplied alpha.
let pixels = leptoskit_canvas::render_to_rgba(&scene, 384, 256, 2.0)?;
```

To draw every frame, create one `Renderer` with your wgpu device, queue, and the target's texture format, and call `render` with the target's view, its size in pixels, and the pixels-per-point scale.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_canvas)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
