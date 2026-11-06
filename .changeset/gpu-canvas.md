---
"leptoskit_canvas": minor
---

# A GPU canvas for custom drawing

The new `leptoskit_canvas` crate draws the kind of graphics no native widget offers, such as charts, progress rings, and cards with soft shadows and gradients. You describe the drawing as a `Scene`, an ordered list of commands in the style of React Native Skia, and its renderer draws it on the GPU with wgpu (Metal on Apple platforms, Vulkan on Linux and Android, Direct3D 12 on Windows, and WebGPU in the browser).

A scene can fill rectangles with a radius per corner and an optional border, cast CSS-style box shadows, paint with solid colours or linear and radial gradients, fill vector paths with the non-zero or even-odd rule, stroke them with caps and joins, and nest clips, transforms, and layers drawn at an opacity. Edges stay smooth at any scale. A scene is plain data, so a Wasm guest can build one and send it to its host, and the renderer bounds every value before drawing, so a scene from untrusted code cannot make the host allocate without limit.

The crate is standalone in this release: no component uses it yet. `render_to_rgba` draws a scene offscreen and returns its pixels, which suits tests and generated images.
