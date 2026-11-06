// The leptoskit canvas shaders: instanced quads that shade rounded rectangles,
// borders, and box shadows analytically from signed distances, triangle meshes
// for tessellated paths, stencil-only meshes for clips, and the quad that blends
// a finished layer into its parent.
//
// `sd_rounded`, `gaussian`, `erf2`, `shadow_x`, and `shadow` are adapted from
// Rune's uber shader (crates/rune/src/shader.wgsl and clip.wgsl), which carries
// this notice:
//
//   Copyright (c) 2026 Anomaly
//
//   Permission is hereby granted, free of charge, to any person obtaining a copy
//   of this software and associated documentation files (the "Software"), to deal
//   in the Software without restriction, including without limitation the rights
//   to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//   copies of the Software, and to permit persons to whom the Software is
//   furnished to do so, subject to the following conditions:
//
//   The above copyright notice and this permission notice shall be included in
//   all copies or substantial portions of the Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//   IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//   FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//   LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//   SOFTWARE.
//
// Colours are sRGB-encoded with premultiplied alpha and blend the way browsers
// blend them. On an sRGB target, `output` decodes them first, so the hardware's
// encode gives the same colours back (blending then happens in linear light).

struct Globals {
	viewport: vec2<f32>,
	srgb: f32,
	pad: f32,
};

// A fill: solid (kind 0), linear gradient (1), or radial gradient (2). A
// gradient's colours are row `row` of the ramp texture; `geom` holds the start
// and the direction scaled by 1/length² (linear), or the centre and 1/radius
// (radial).
struct Paint {
	kind: u32,
	row: u32,
	pad: vec2<u32>,
	color: vec4<f32>,
	geom: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> paints: array<Paint>;
@group(0) @binding(2) var ramps: texture_2d<f32>;
@group(0) @binding(3) var ramp_sampler: sampler;

fn to_clip(screen: vec2<f32>) -> vec4<f32> {
	return vec4<f32>(screen.x / g.viewport.x * 2.0 - 1.0, 1.0 - screen.y / g.viewport.y * 2.0, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
	return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn output(c: vec4<f32>) -> vec4<f32> {
	if (g.srgb < 0.5 || c.a <= 0.0) {
		return c;
	}
	return vec4<f32>(srgb_to_linear(clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0))) * c.a, c.a);
}

fn paint(id: u32, p: vec2<f32>) -> vec4<f32> {
	let r = paints[id];
	if (r.kind == 0u) {
		return r.color;
	}
	var t: f32;
	if (r.kind == 1u) {
		t = dot(p - r.geom.xy, r.geom.zw);
	} else {
		t = length(p - r.geom.xy) * r.geom.z;
	}
	let u = (clamp(t, 0.0, 1.0) * 255.0 + 0.5) / 256.0;
	let v = (f32(r.row) + 0.5) / f32(textureDimensions(ramps).y);
	return textureSampleLevel(ramps, ramp_sampler, vec2<f32>(u, v), 0.0);
}

// Signed distance from `p` (relative to the centre) to a rounded rectangle
// with half-size `half`; radii run top-left, top-right, bottom-right,
// bottom-left with y down.
fn sd_rounded(p: vec2<f32>, half: vec2<f32>, radii: vec4<f32>) -> f32 {
	var r: f32;
	if (p.x > 0.0) {
		r = select(radii.y, radii.z, p.y > 0.0);
	} else {
		r = select(radii.x, radii.w, p.y > 0.0);
	}
	r = min(r, min(half.x, half.y));
	let q = abs(p) - half + vec2<f32>(r);
	return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

// Evan Wallace's analytic rounded-box shadow: exact along x, 4 samples along y.
fn gaussian(x: f32, sigma: f32) -> f32 {
	return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * 3.14159265) * sigma);
}

fn erf2(x: vec2<f32>) -> vec2<f32> {
	let s = sign(x);
	let a = abs(x);
	var r = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
	r = r * r;
	return s - s / (r * r);
}

fn shadow_x(x: f32, y: f32, sigma: f32, corner: f32, half: vec2<f32>) -> f32 {
	let delta = min(half.y - corner - abs(y), 0.0);
	let curved = half.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
	let integral = 0.5 + 0.5 * erf2((x + vec2<f32>(-curved, curved)) * (sqrt(0.5) / sigma));
	return integral.y - integral.x;
}

// What `shadow` sums to well inside the box: the four midpoint weights of the
// ±3σ window.
const SHADOW_INSIDE: f32 = 0.99863125;

fn shadow(p: vec2<f32>, half: vec2<f32>, sigma: f32, corner: f32) -> f32 {
	if (abs(p.x) <= half.x - 6.0 * sigma && abs(p.y) <= half.y - corner - 6.0 * sigma) {
		return SHADOW_INSIDE;
	}
	let lo = p.y - half.y;
	let hi = p.y + half.y;
	let start = clamp(-3.0 * sigma, lo, hi);
	let end = clamp(3.0 * sigma, lo, hi);
	let step = (end - start) / 4.0;
	var y = start + step * 0.5;
	var v = 0.0;
	for (var i = 0; i < 4; i = i + 1) {
		v = v + shadow_x(p.x, p.y - y, sigma, corner, half) * gaussian(y, sigma) * step;
		y = y + step;
	}
	return v;
}

// One rounded rectangle (kind 0) or box shadow (kind 1) in its own coordinate
// space. `m0` and `m1.xy` map that space to device pixels, `m1.z` is how many
// pixels one unit covers, and `m1.w` is how far the quad reaches past `rect`.
struct QuadIn {
	@builtin(vertex_index) vi: u32,
	@location(0) rect: vec4<f32>,
	@location(1) radii: vec4<f32>,
	@location(2) m0: vec4<f32>,
	@location(3) m1: vec4<f32>,
	// kind, border width, shadow sigma, unused.
	@location(4) params: vec4<f32>,
	// The border or shadow colour, premultiplied.
	@location(5) color: vec4<f32>,
	@location(6) paint: u32,
};

struct QuadOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) p: vec2<f32>,
	@location(1) @interpolate(flat) rect: vec4<f32>,
	@location(2) @interpolate(flat) radii: vec4<f32>,
	@location(3) @interpolate(flat) params: vec4<f32>,
	@location(4) @interpolate(flat) color: vec4<f32>,
	@location(5) @interpolate(flat) paint: u32,
	@location(6) @interpolate(flat) px: f32,
};

@vertex
fn vs_quad(i: QuadIn) -> QuadOut {
	let corner = vec2<f32>(f32(i.vi & 1u), f32(i.vi >> 1u));
	let lo = i.rect.xy - vec2<f32>(i.m1.w);
	let hi = i.rect.xy + i.rect.zw + vec2<f32>(i.m1.w);
	let p = mix(lo, hi, corner);
	var o: QuadOut;
	o.pos = to_clip(vec2<f32>(i.m0.x * p.x + i.m0.z * p.y + i.m1.x, i.m0.y * p.x + i.m0.w * p.y + i.m1.y));
	o.p = p;
	o.rect = i.rect;
	o.radii = i.radii;
	o.params = i.params;
	o.color = i.color;
	o.paint = i.paint;
	o.px = i.m1.z;
	return o;
}

@fragment
fn fs_quad(i: QuadOut) -> @location(0) vec4<f32> {
	let half = i.rect.zw * 0.5;
	let q = i.p - (i.rect.xy + half);
	let d = sd_rounded(q, half, i.radii);
	let cov = clamp(0.5 - d * i.px, 0.0, 1.0);
	if (i.params.x > 0.5) {
		let sigma = i.params.z;
		if (sigma * i.px < 0.25) {
			return output(i.color * cov);
		}
		var corner: f32;
		if (q.x > 0.0) {
			corner = select(i.radii.y, i.radii.z, q.y > 0.0);
		} else {
			corner = select(i.radii.x, i.radii.w, q.y > 0.0);
		}
		return output(i.color * shadow(q, half, sigma, min(corner, min(half.x, half.y))));
	}
	let fill = paint(i.paint, i.p);
	let bw = i.params.y;
	if (bw > 0.0) {
		let inner = clamp(0.5 - (d + bw) * i.px, 0.0, 1.0);
		return output(fill * inner + i.color * (cov - inner));
	}
	return output(fill * cov);
}

// A tessellated path vertex: already in device pixels, with its point in the
// path's own space for the paint.
struct MeshIn {
	@location(0) pos: vec2<f32>,
	@location(1) local: vec2<f32>,
	@location(2) paint: u32,
};

struct MeshOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) local: vec2<f32>,
	@location(1) @interpolate(flat) paint: u32,
};

@vertex
fn vs_mesh(i: MeshIn) -> MeshOut {
	var o: MeshOut;
	o.pos = to_clip(i.pos);
	o.local = i.local;
	o.paint = i.paint;
	return o;
}

@fragment
fn fs_mesh(i: MeshOut) -> @location(0) vec4<f32> {
	return output(paint(i.paint, i.local));
}

// Clips write only the stencil buffer.
@fragment
fn fs_clip() -> @location(0) vec4<f32> {
	return vec4<f32>(0.0);
}

// A finished layer, the size of the target, blended at `opacity`.
@group(1) @binding(0) var layer: texture_2d<f32>;

struct CompositeOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) @interpolate(flat) opacity: f32,
};

@vertex
fn vs_composite(@builtin(vertex_index) vi: u32, @location(0) opacity: f32) -> CompositeOut {
	let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
	var o: CompositeOut;
	o.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
	o.opacity = opacity;
	return o;
}

@fragment
fn fs_composite(i: CompositeOut) -> @location(0) vec4<f32> {
	return textureLoad(layer, vec2<i32>(i.pos.xy), 0) * i.opacity;
}
