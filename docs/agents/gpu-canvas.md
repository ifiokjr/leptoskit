# GPU canvas

`leptoskit_canvas` is a GPU drawing surface in the style of React Native Skia: a `Scene` (an ordered display list) and a wgpu renderer. This note records what Phase A built and how the later phases host it. The crate's own contract is in `crates/leptoskit_canvas/readme.md`.

## Phase A: the crate (done)

The crate stands alone. No other crate depends on it, and it is not re-exported from `leptoskit`, so nothing ships to apps yet.

### Display list

`Scene { commands: Vec<Command> }`, serde-optional, in logical points with y down:

| Command                    | What it draws                                                                                                             |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `FillRect`                 | A rectangle with a radius per corner, a `Paint`, and an optional border drawn inside the edge (`box-sizing: border-box`). |
| `BoxShadow`                | A CSS `box-shadow` (offset, blur, spread, colour); the shadow also covers the area under the box.                         |
| `FillPath`, `StrokePath`   | A `Path` (move, line, quad, cubic, close) filled non-zero or even-odd, or stroked with width, cap, join, miter limit.     |
| `PushClip`, `PushClipPath` | A clip to a rounded rectangle or a path; nested clips intersect.                                                          |
| `PushLayer { opacity }`    | Draws into a texture of its own, blended into its parent at `opacity` when popped.                                        |
| `PushTransform(Affine)`    | An affine transform applied after the ones already open (canvas `transform` order).                                       |

`Paint` is solid, linear gradient (start, end, stops), or radial gradient (centre, radius, stops). `Color` is sRGB with straight alpha; the renderer premultiplies, and gradients mix premultiplied, as CSS does.

### Untrusted input

`Scene::sanitized` is what a host applies to a guest's scene, and `Renderer::render` always applies it. It caps commands (10 000), path commands per path (10 000) and per scene (100 000), gradient stops (16) and gradients (1 024, later ones become their first colour), nesting (32 open pushes, 8 layers), and clamps every coordinate to ±10⁶, transform scales to ±10⁴, blur to 1 000, stroke width to 10 000, and miter limit to [1, 100]. Non-finite values become the bound nearest zero. Pops that do not match the innermost push are dropped and unclosed pushes are closed. The encoder also skips anything that lands off the target, flattens curves no finer than 1/4 096 of the path's own size (so zooming cannot explode the vertex count), and stops adding paths past 2 000 000 vertices. The renderer clamps the pixels-per-point scale to 16 and rejects targets larger than the device's texture limit.

### Renderer

- **Quads.** `FillRect` and `BoxShadow` become one 112-byte instance each (rect, radii, a 2×3 transform with pixels-per-unit and quad margin, kind, border width, shadow sigma, border or shadow colour, paint index), drawn as an instanced triangle strip. The fragment shader computes the rounded rectangle's signed distance in the shape's own space and turns it into coverage using the pixels-per-unit, so rotated and scaled rectangles stay smooth. Shadows use Evan Wallace's analytic rounded-box Gaussian (exact along x, four samples along y).
- **Paths.** `lyon` tessellates fills (with their fill rule, so holes need no stencil) and strokes in the path's own space; vertices are transformed to device pixels on the CPU and keep their local position for the paint.
- **Anti-aliasing.** Everything draws into a 4× multisampled target. Quads are anti-aliased analytically and their quads reach a pixel past the shape, so multisampling adds nothing to them; path edges and clip edges get 4× multisampling.
- **Paints.** A storage buffer of 48-byte paint records (kind, ramp row, premultiplied colour, gradient geometry). Gradient stops are baked into rows of a 256-texel RGBA8 ramp texture and sampled linearly.
- **Clips.** A stencil buffer (`Stencil8`, multisampled). A push draws the clip's triangles with `compare = Equal(depth)`, `pass = IncrementClamp`; content draws with `Equal(depth)`; a pop draws the same triangles with `DecrementClamp`. This handles rounded, path, and transformed clips with one mechanism.
- **Layers.** The encoder splits the frame into render passes wherever a layer opens or closes. Each nesting level has its own multisampled colour and stencil textures (pooled across frames) and a resolve texture; closing a layer resumes the parent pass with `LoadOp::Load` and draws a full-screen triangle that loads the layer texture and multiplies by its opacity, under the parent's stencil.
- **Colour.** Colours are sRGB-encoded and blended as browsers blend them. On an sRGB target the shader decodes before writing, which shows the same colours but blends in linear light.
- **Backends.** wgpu 30 with only the backend each platform needs: Metal on Apple, Vulkan on Linux and Android, Direct3D 12 on Windows, and WebGPU on wasm32. There is no GL fallback yet. The WGSL is compiled at runtime by naga; nothing is precompiled. `render_to_rgba` and `headless_device` (pollster) exist only off wasm32.

### What came from Rune

[Rune](https://github.com/anomalyco/rune) (MIT, Copyright (c) 2026 Anomaly) informed the record-and-uber-shader design: one instance record per primitive, a kind field, flat-interpolated parameters, and analytic coverage from signed distances. `sd_rounded`, `gaussian`, `erf2`, `shadow_x`, and `shadow` in `shader.wgsl` are adapted from Rune's `shader.wgsl` and `clip.wgsl`; the notice is kept in the shader header and in `crates/leptoskit_canvas/LICENSE-rune`. Rune's analytic clip rectangles, continuous (squircle) corners, glyph atlas, images, and spring tracks were not taken. Rune has no general path engine, so paths use `lyon` and multisampling instead of Rune's per-triangle edge coverage.

### Tests

`tests/render.rs` renders small scenes headlessly and checks pixels: solid rectangles, rounded corners, borders, linear and radial gradients, shadow falloff, offset, and spread, even-odd holes, curve anti-aliasing, stroke width and caps, clips (rectangular, rounded, nested), layer opacity, transforms, scale, a BGRA renderer reused across sizes, and a hostile scene. Without a GPU adapter, `render_to_rgba` returns an error whose `is_unavailable()` is true and each test prints `skipping:` and passes; CI Linux runners without a GPU (or without lavapipe) skip them. `tests/sanitize.rs` and the encoder's unit tests need no GPU.

## Phase B: the `Canvas` element and the web

- A `Canvas` component in `leptoskit_native` carrying a `Scene` prop (the crate with `default-features = false`, so guests do not build wgpu). The prop needs a compact binary encoding for the guest ABI rather than JSON, and a way to send only what changed (for example, keyed sub-scenes) so an animated chart does not resend every path.
- Layout: the element is a leaf sized by Tailwind like a `View`; the scene's points map to its frame, and the backend supplies the scale.
- Web: render into an HTML `<canvas>` with wgpu's WebGPU backend. Browsers without WebGPU need a fallback: wgpu's WebGL2 backend (needs the paint storage buffer moved to a texture or uniform array, since WebGL2 has no fragment storage buffers, and a ramp texture height within 2 048), or a Canvas2D interpreter of the same display list, which is small and matches the browser's own anti-aliasing. Decide after measuring how much wgpu adds to the browser bundle.
- `docs/src`: a guide page for app authors with a concrete example and per-platform behaviour, and update "Where things stand".

## Phase C: Apple hosting

- A Swift `CanvasView` in `leptoskit_bridge_ffi` backed by a `CAMetalLayer` (`framebufferOnly`, `presentsWithTransaction` when it must sync with UIKit/AppKit layout, `contentsScale` from the screen).
- Rust owns the wgpu device and renderer behind the bridge FFI: create a wgpu surface from the layer (`SurfaceTargetUnsafe::CoreAnimationLayer`), resize on layout, and render when the scene prop changes. One device per process, shared by every canvas.
- Pacing with `CADisplayLink` (on macOS 14 and later, from `NSView.displayLink(target:selector:)`), only while something animates, with the same subscription rules as Rive in [rive.md](rive.md).
- The guest thread work in [guest-thread.md](guest-thread.md) decides which thread encodes; presenting stays on the main thread or a dedicated render thread.
- Later, move native Rive onto this surface: draw Rive's vector output as canvas commands (or a texture) instead of uploading CPU-rastered RGBA each frame.

## Phase D: Android, GPU animation, and text

- Android: a `SurfaceView` (or `TextureView` when it must transform with the view tree) whose `Surface` becomes an `ANativeWindow` for a Vulkan wgpu surface; frames paced by `Choreographer`. Devices without Vulkan 1.1 need wgpu's GLES backend, so add the `gles` feature for Android only, and keep the WGSL within GLES limits.
- Animating on the GPU without resending the scene: Rune keeps "spring tracks" (`crates/rune/src/gpu/springs.rs`), per-instance translation and opacity springs whose closed-form coefficients are computed once on retarget and evaluated in the vertex shader from the frame time. The same idea fits here: the guest sends a scene once plus animation tracks keyed to instance or command ids, and each frame updates only a time uniform. Hit testing samples the same closed form on the CPU.
- Text: shape with `parley` (or `swash` directly), rasterize glyphs into an atlas texture, and draw glyph quads with a new instance kind, keeping colour glyphs (emoji) in an RGBA atlas. Until then, text stays in native `Text` views layered over the canvas.
- Images (an RGBA atlas kind), blur and backdrop filters (Rune's `layer_blur` and `backdrop_blur` passes are the reference), dashed strokes (`lyon` has no dashing; flatten and split), and conic gradients are further additions.
