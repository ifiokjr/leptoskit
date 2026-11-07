//! The wgpu renderer.

use wgpu::util::DeviceExt as _;

use crate::encode::Draw;
use crate::encode::Frame;
use crate::encode::Instance;
use crate::encode::MeshVertex;
use crate::encode::RAMP_WIDTH;
use crate::encode::encode;
use crate::model::Scene;

/// Samples per pixel. Paths and clip edges are smoothed by multisampling;
/// rectangles and shadows are smoothed in the shader.
const SAMPLES: u32 = 4;
const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;
/// The largest pixels-per-point scale the renderer accepts.
const MAX_SCALE: f32 = 16.0;

/// Why a canvas could not be drawn.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The system has no GPU adapter wgpu can use.
	#[error("no GPU adapter is available: {0}")]
	NoAdapter(String),
	/// The adapter refused to create a device.
	#[error("the GPU device could not be created: {0}")]
	Device(String),
	/// The canvas is bigger than the GPU's largest texture.
	#[error("the canvas is {width}×{height} pixels, larger than the GPU limit of {limit}")]
	TooLarge { width: u32, height: u32, limit: u32 },
	/// Pixels can only be read back from 8-bit RGBA or BGRA formats.
	#[error("pixels cannot be read back from the {0:?} format")]
	UnsupportedFormat(wgpu::TextureFormat),
	/// Reading the pixels back from the GPU failed.
	#[error("reading the pixels back failed: {0}")]
	Readback(String),
}

impl Error {
	/// Whether the error means there is no usable GPU, rather than a
	/// problem with the scene or target.
	pub fn is_unavailable(&self) -> bool {
		matches!(self, Self::NoAdapter(_) | Self::Device(_))
	}
}

/// A GPU buffer that grows to fit what it is given.
struct Growable {
	label: &'static str,
	usage: wgpu::BufferUsages,
	buffer: Option<wgpu::Buffer>,
}

impl Growable {
	fn new(label: &'static str, usage: wgpu::BufferUsages) -> Self {
		Self {
			label,
			usage: usage | wgpu::BufferUsages::COPY_DST,
			buffer: None,
		}
	}

	/// Uploads `data`, returning whether the buffer had to be replaced.
	fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) -> bool {
		if data.is_empty() {
			return false;
		}
		let size = data.len() as u64;
		let replaced = self
			.buffer
			.as_ref()
			.is_none_or(|buffer| buffer.size() < size);
		if replaced {
			self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
				label: Some(self.label),
				size: size.next_power_of_two().max(256),
				usage: self.usage,
				mapped_at_creation: false,
			}));
		}
		if let Some(buffer) = &self.buffer {
			queue.write_buffer(buffer, 0, data);
		}
		replaced
	}

	fn slice(&self) -> Option<wgpu::BufferSlice<'_>> {
		self.buffer.as_ref().map(|buffer| buffer.slice(..))
	}
}

/// The textures one level of drawing needs: the target (level 0) or a
/// layer.
struct Level {
	msaa: wgpu::TextureView,
	stencil: wgpu::TextureView,
	/// A layer's finished pixels and the bind group that samples them.
	/// Level 0 resolves into the caller's target instead.
	resolve: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

/// Draws [`Scene`]s with wgpu into textures of one format.
///
/// It keeps its pipelines, buffers, and layer textures between frames, so
/// keep one renderer per surface and call [`Renderer::render`] each frame.
pub struct Renderer {
	device: wgpu::Device,
	queue: wgpu::Queue,
	format: wgpu::TextureFormat,
	quad: wgpu::RenderPipeline,
	mesh: wgpu::RenderPipeline,
	clip_push: wgpu::RenderPipeline,
	clip_pop: wgpu::RenderPipeline,
	composite: wgpu::RenderPipeline,
	globals_layout: wgpu::BindGroupLayout,
	layer_layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	globals: wgpu::Buffer,
	instances: Growable,
	vertices: Growable,
	indices: Growable,
	paints: Growable,
	opacities: Growable,
	ramps: Option<(wgpu::Texture, u32)>,
	bind_group: Option<wgpu::BindGroup>,
	levels: Vec<Level>,
	size: (u32, u32),
}

fn stencil(
	compare: wgpu::CompareFunction,
	pass_op: wgpu::StencilOperation,
) -> wgpu::DepthStencilState {
	let face = wgpu::StencilFaceState {
		compare,
		fail_op: wgpu::StencilOperation::Keep,
		depth_fail_op: wgpu::StencilOperation::Keep,
		pass_op,
	};
	wgpu::DepthStencilState {
		format: STENCIL_FORMAT,
		depth_write_enabled: None,
		depth_compare: None,
		stencil: wgpu::StencilState {
			front: face,
			back: face,
			read_mask: 0xff,
			write_mask: 0xff,
		},
		bias: wgpu::DepthBiasState::default(),
	}
}

/// The layouts of the globals bind group, shared by every pipeline, and of
/// a finished layer's texture, used by the composite pipeline.
fn bind_group_layouts(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
	let fragment = wgpu::ShaderStages::FRAGMENT;
	let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("leptoskit_canvas globals"),
		entries: &[
			wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX | fragment,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			},
			wgpu::BindGroupLayoutEntry {
				binding: 1,
				visibility: fragment,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Storage { read_only: true },
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			},
			wgpu::BindGroupLayoutEntry {
				binding: 2,
				visibility: fragment,
				ty: wgpu::BindingType::Texture {
					sample_type: wgpu::TextureSampleType::Float { filterable: true },
					view_dimension: wgpu::TextureViewDimension::D2,
					multisampled: false,
				},
				count: None,
			},
			wgpu::BindGroupLayoutEntry {
				binding: 3,
				visibility: fragment,
				ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
				count: None,
			},
		],
	});
	let layer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("leptoskit_canvas layer"),
		entries: &[wgpu::BindGroupLayoutEntry {
			binding: 0,
			visibility: fragment,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: false },
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		}],
	});
	(globals_layout, layer_layout)
}

/// The parts of a pipeline that differ between the renderer's pipelines.
struct PipelineSpec<'a> {
	label: &'static str,
	layout: &'a wgpu::PipelineLayout,
	vertex: &'static str,
	fragment: &'static str,
	buffer: wgpu::VertexBufferLayout<'a>,
	topology: wgpu::PrimitiveTopology,
	stencil: wgpu::DepthStencilState,
	color: bool,
}

/// Creates one of the renderer's pipelines; they differ only in what
/// `spec` holds.
fn create_pipeline(
	device: &wgpu::Device,
	shader: &wgpu::ShaderModule,
	format: wgpu::TextureFormat,
	spec: PipelineSpec<'_>,
) -> wgpu::RenderPipeline {
	let targets = [Some(wgpu::ColorTargetState {
		format,
		blend: spec
			.color
			.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
		write_mask: if spec.color {
			wgpu::ColorWrites::ALL
		} else {
			wgpu::ColorWrites::empty()
		},
	})];
	device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some(spec.label),
		layout: Some(spec.layout),
		vertex: wgpu::VertexState {
			module: shader,
			entry_point: Some(spec.vertex),
			compilation_options: wgpu::PipelineCompilationOptions::default(),
			buffers: &[Some(spec.buffer)],
		},
		primitive: wgpu::PrimitiveState {
			topology: spec.topology,
			..wgpu::PrimitiveState::default()
		},
		depth_stencil: Some(spec.stencil),
		multisample: wgpu::MultisampleState {
			count: SAMPLES,
			..wgpu::MultisampleState::default()
		},
		fragment: Some(wgpu::FragmentState {
			module: shader,
			entry_point: Some(spec.fragment),
			compilation_options: wgpu::PipelineCompilationOptions::default(),
			targets: &targets,
		}),
		multiview_mask: None,
		cache: None,
	})
}

impl Renderer {
	/// A renderer that draws into textures of `format` on `device`.
	///
	/// Use a format without sRGB encoding (such as `Rgba8Unorm` or
	/// `Bgra8Unorm`) to blend colours the way browsers do. An sRGB format
	/// shows the same colours but blends them in linear light.
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("leptoskit_canvas shader"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
		});
		let (globals_layout, layer_layout) = bind_group_layouts(device);
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("leptoskit_canvas"),
			bind_group_layouts: &[Some(&globals_layout)],
			immediate_size: 0,
		});
		let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("leptoskit_canvas composite"),
			bind_group_layouts: &[Some(&globals_layout), Some(&layer_layout)],
			immediate_size: 0,
		});
		let quad_attributes = wgpu::vertex_attr_array![
			0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
			4 => Float32x4, 5 => Float32x4, 6 => Uint32,
		];
		let mesh_attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32];
		let composite_attributes = wgpu::vertex_attr_array![0 => Float32];
		let mesh_buffer = wgpu::VertexBufferLayout {
			array_stride: size_of::<MeshVertex>() as u64,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &mesh_attributes,
		};
		let keep = wgpu::StencilOperation::Keep;
		let equal = wgpu::CompareFunction::Equal;
		let pipeline = |spec| create_pipeline(device, &shader, format, spec);
		let quad = pipeline(PipelineSpec {
			label: "leptoskit_canvas quads",
			layout: &layout,
			vertex: "vs_quad",
			fragment: "fs_quad",
			buffer: wgpu::VertexBufferLayout {
				array_stride: size_of::<Instance>() as u64,
				step_mode: wgpu::VertexStepMode::Instance,
				attributes: &quad_attributes,
			},
			topology: wgpu::PrimitiveTopology::TriangleStrip,
			stencil: stencil(equal, keep),
			color: true,
		});
		let mesh_spec = |label, stencil, color| {
			PipelineSpec {
				label,
				layout: &layout,
				vertex: "vs_mesh",
				fragment: if color { "fs_mesh" } else { "fs_clip" },
				buffer: mesh_buffer.clone(),
				topology: wgpu::PrimitiveTopology::TriangleList,
				stencil,
				color,
			}
		};
		let mesh = pipeline(mesh_spec(
			"leptoskit_canvas paths",
			stencil(equal, keep),
			true,
		));
		let clip_push = pipeline(mesh_spec(
			"leptoskit_canvas clip push",
			stencil(equal, wgpu::StencilOperation::IncrementClamp),
			false,
		));
		let clip_pop = pipeline(mesh_spec(
			"leptoskit_canvas clip pop",
			stencil(equal, wgpu::StencilOperation::DecrementClamp),
			false,
		));
		let composite = pipeline(PipelineSpec {
			label: "leptoskit_canvas composite",
			layout: &composite_layout,
			vertex: "vs_composite",
			fragment: "fs_composite",
			buffer: wgpu::VertexBufferLayout {
				array_stride: size_of::<f32>() as u64,
				step_mode: wgpu::VertexStepMode::Instance,
				attributes: &composite_attributes,
			},
			topology: wgpu::PrimitiveTopology::TriangleList,
			stencil: stencil(equal, keep),
			color: true,
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("leptoskit_canvas ramps"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			..wgpu::SamplerDescriptor::default()
		});
		let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("leptoskit_canvas globals"),
			contents: &[0; 16],
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
		});
		Self {
			device: device.clone(),
			queue: queue.clone(),
			format,
			quad,
			mesh,
			clip_push,
			clip_pop,
			composite,
			globals_layout,
			layer_layout,
			sampler,
			globals,
			instances: Growable::new("leptoskit_canvas instances", wgpu::BufferUsages::VERTEX),
			vertices: Growable::new("leptoskit_canvas vertices", wgpu::BufferUsages::VERTEX),
			indices: Growable::new("leptoskit_canvas indices", wgpu::BufferUsages::INDEX),
			paints: Growable::new("leptoskit_canvas paints", wgpu::BufferUsages::STORAGE),
			opacities: Growable::new("leptoskit_canvas opacities", wgpu::BufferUsages::VERTEX),
			ramps: None,
			bind_group: None,
			levels: Vec::new(),
			size: (0, 0),
		}
	}

	/// The texture format the renderer draws into.
	pub fn format(&self) -> wgpu::TextureFormat {
		self.format
	}

	/// Draws `scene` into `target`, a single-sampled view `width` by
	/// `height` pixels in the renderer's format, replacing what it held
	/// with transparency first. `scale` is how many pixels make one point
	/// of the scene, such as 2 on a Retina display.
	///
	/// The scene is sanitized first, so it may come from untrusted code.
	pub fn render(
		&mut self,
		scene: &Scene,
		target: &wgpu::TextureView,
		width: u32,
		height: u32,
		scale: f32,
	) -> Result<(), Error> {
		if width == 0 || height == 0 {
			return Ok(());
		}
		let limit = self.device.limits().max_texture_dimension_2d;
		if width > limit || height > limit {
			return Err(Error::TooLarge {
				width,
				height,
				limit,
			});
		}
		let scale = if scale.is_finite() && scale > 0.0 {
			scale.min(MAX_SCALE)
		} else {
			1.0
		};
		let frame = encode(&scene.sanitized(), width, height, scale);
		self.prepare(&frame, width, height);
		let mut encoder = self
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("leptoskit_canvas"),
			});
		self.record(&mut encoder, &frame, target);
		self.queue.submit([encoder.finish()]);
		Ok(())
	}

	/// Uploads the frame's buffers and makes sure every level's textures
	/// exist at the target's size.
	fn prepare(&mut self, frame: &Frame, width: u32, height: u32) {
		let srgb = if self.format.is_srgb() { 1.0 } else { 0.0 };
		let globals: [f32; 4] = [width as f32, height as f32, srgb, 0.0];
		self.queue
			.write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));
		let (device, queue) = (&self.device, &self.queue);
		self.instances
			.upload(device, queue, bytemuck::cast_slice(&frame.instances));
		self.vertices
			.upload(device, queue, bytemuck::cast_slice(&frame.mesh.vertices));
		self.indices
			.upload(device, queue, bytemuck::cast_slice(&frame.mesh.indices));
		self.opacities
			.upload(device, queue, bytemuck::cast_slice(&frame.opacities));
		let mut stale = self
			.paints
			.upload(device, queue, bytemuck::cast_slice(&frame.paints));
		stale |= self.upload_ramps(frame);
		if stale || self.bind_group.is_none() {
			self.bind_group = self.globals_bind_group();
		}
		if self.size != (width, height) {
			self.levels.clear();
			self.size = (width, height);
		}
		while self.levels.len() < frame.levels {
			let level = self.level(self.levels.len());
			self.levels.push(level);
		}
	}

	/// Writes the gradient ramps, returning whether the texture was
	/// replaced.
	fn upload_ramps(&mut self, frame: &Frame) -> bool {
		let rows = frame.ramp_rows().max(1) as u32;
		let replaced = self
			.ramps
			.as_ref()
			.is_none_or(|(_, capacity)| *capacity < rows);
		if replaced {
			let capacity = rows.next_power_of_two();
			let texture = self.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("leptoskit_canvas ramps"),
				size: wgpu::Extent3d {
					width: RAMP_WIDTH as u32,
					height: capacity,
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			self.ramps = Some((texture, capacity));
		}
		if let Some((texture, _)) = &self.ramps
			&& !frame.ramps.is_empty()
		{
			self.queue.write_texture(
				texture.as_image_copy(),
				&frame.ramps,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(RAMP_WIDTH as u32 * 4),
					rows_per_image: None,
				},
				wgpu::Extent3d {
					width: RAMP_WIDTH as u32,
					height: frame.ramp_rows() as u32,
					depth_or_array_layers: 1,
				},
			);
		}
		replaced
	}

	fn globals_bind_group(&self) -> Option<wgpu::BindGroup> {
		let paints = self.paints.buffer.as_ref()?;
		let (ramps, _) = self.ramps.as_ref()?;
		let ramps = ramps.create_view(&wgpu::TextureViewDescriptor::default());
		Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("leptoskit_canvas globals"),
			layout: &self.globals_layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: self.globals.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: paints.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&ramps),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::Sampler(&self.sampler),
				},
			],
		}))
	}

	/// Creates the textures for drawing level `index`.
	fn level(&self, index: usize) -> Level {
		let (width, height) = self.size;
		let texture = |label, format, samples, usage| {
			self.device
				.create_texture(&wgpu::TextureDescriptor {
					label: Some(label),
					size: wgpu::Extent3d {
						width,
						height,
						depth_or_array_layers: 1,
					},
					mip_level_count: 1,
					sample_count: samples,
					dimension: wgpu::TextureDimension::D2,
					format,
					usage,
					view_formats: &[],
				})
				.create_view(&wgpu::TextureViewDescriptor::default())
		};
		let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
		let resolve = (index > 0).then(|| {
			let view = texture(
				"leptoskit_canvas layer",
				self.format,
				1,
				attachment | wgpu::TextureUsages::TEXTURE_BINDING,
			);
			let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("leptoskit_canvas layer"),
				layout: &self.layer_layout,
				entries: &[wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&view),
				}],
			});
			(view, bind_group)
		});
		Level {
			msaa: texture("leptoskit_canvas samples", self.format, SAMPLES, attachment),
			stencil: texture(
				"leptoskit_canvas stencil",
				STENCIL_FORMAT,
				SAMPLES,
				attachment,
			),
			resolve,
		}
	}

	/// Records every render pass of the frame.
	fn record(
		&self,
		encoder: &mut wgpu::CommandEncoder,
		frame: &Frame,
		target: &wgpu::TextureView,
	) {
		let Some(bind_group) = &self.bind_group else {
			return;
		};
		for segment in &frame.segments {
			let level = &self.levels[segment.level];
			let resolve = level.resolve.as_ref().map_or(target, |(view, _)| view);
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("leptoskit_canvas"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &level.msaa,
					depth_slice: None,
					resolve_target: Some(resolve),
					ops: wgpu::Operations {
						load: if segment.clear {
							wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
						} else {
							wgpu::LoadOp::Load
						},
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &level.stencil,
					depth_ops: None,
					stencil_ops: Some(wgpu::Operations {
						load: if segment.clear {
							wgpu::LoadOp::Clear(0)
						} else {
							wgpu::LoadOp::Load
						},
						store: wgpu::StoreOp::Store,
					}),
				}),
				..wgpu::RenderPassDescriptor::default()
			});
			pass.set_bind_group(0, bind_group, &[]);
			for step in &frame.steps[segment.steps.clone()] {
				pass.set_stencil_reference(step.stencil);
				self.draw(&mut pass, &step.draw);
			}
		}
	}

	fn draw(&self, pass: &mut wgpu::RenderPass<'_>, draw: &Draw) {
		match draw {
			Draw::Quads(range) => {
				if let Some(instances) = self.instances.slice() {
					pass.set_pipeline(&self.quad);
					pass.set_vertex_buffer(0, instances);
					pass.draw(0..4, range.clone());
				}
			}
			Draw::Mesh(range) | Draw::ClipPush(range) | Draw::ClipPop(range) => {
				let (Some(vertices), Some(indices)) = (self.vertices.slice(), self.indices.slice())
				else {
					return;
				};
				if range.is_empty() {
					return;
				}
				pass.set_pipeline(match draw {
					Draw::ClipPush(_) => &self.clip_push,
					Draw::ClipPop(_) => &self.clip_pop,
					_ => &self.mesh,
				});
				pass.set_vertex_buffer(0, vertices);
				pass.set_index_buffer(indices, wgpu::IndexFormat::Uint32);
				pass.draw_indexed(range.clone(), 0, 0..1);
			}
			Draw::Composite { level, instance } => {
				let (Some(opacities), Some((_, layer))) =
					(self.opacities.slice(), self.levels[*level].resolve.as_ref())
				else {
					return;
				};
				pass.set_pipeline(&self.composite);
				pass.set_bind_group(1, layer, &[]);
				pass.set_vertex_buffer(0, opacities);
				pass.draw(0..3, *instance..*instance + 1);
			}
		}
	}

	/// Draws `scene` offscreen and reads it back as RGBA bytes, four per
	/// pixel, row by row, with premultiplied alpha. The renderer's format
	/// must be 8-bit RGBA or BGRA.
	#[cfg(not(target_arch = "wasm32"))]
	pub fn render_to_rgba(
		&mut self,
		scene: &Scene,
		width: u32,
		height: u32,
		scale: f32,
	) -> Result<Vec<u8>, Error> {
		use wgpu::TextureFormat as F;
		let swap = match self.format {
			F::Rgba8Unorm | F::Rgba8UnormSrgb => false,
			F::Bgra8Unorm | F::Bgra8UnormSrgb => true,
			format => return Err(Error::UnsupportedFormat(format)),
		};
		if width == 0 || height == 0 {
			return Ok(Vec::new());
		}
		let size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};
		let limit = self.device.limits().max_texture_dimension_2d;
		if width > limit || height > limit {
			return Err(Error::TooLarge {
				width,
				height,
				limit,
			});
		}
		let texture = self.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("leptoskit_canvas offscreen"),
			size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: self.format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		});
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		self.render(scene, &view, width, height, scale)?;
		let row = width as usize * 4;
		let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
		let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("leptoskit_canvas readback"),
			size: (padded * height as usize) as u64,
			usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let mut encoder = self
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("leptoskit_canvas readback"),
			});
		encoder.copy_texture_to_buffer(
			texture.as_image_copy(),
			wgpu::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(padded as u32),
					rows_per_image: None,
				},
			},
			size,
		);
		self.queue.submit([encoder.finish()]);
		let (sender, receiver) = std::sync::mpsc::channel();
		buffer.map_async(wgpu::MapMode::Read, .., move |result| {
			let _ = sender.send(result);
		});
		self.device
			.poll(wgpu::PollType::wait_indefinitely())
			.map_err(|error| Error::Readback(error.to_string()))?;
		receiver
			.recv()
			.map_err(|error| Error::Readback(error.to_string()))?
			.map_err(|error| Error::Readback(error.to_string()))?;
		let mapped = buffer
			.get_mapped_range(..)
			.map_err(|error| Error::Readback(error.to_string()))?;
		let mut pixels = Vec::with_capacity(row * height as usize);
		for line in mapped.chunks(padded) {
			pixels.extend_from_slice(&line[..row]);
		}
		drop(mapped);
		buffer.unmap();
		if swap {
			for pixel in pixels.as_chunks_mut::<4>().0 {
				pixel.swap(0, 2);
			}
		}
		Ok(pixels)
	}
}

/// Creates a device with no window, for drawing offscreen.
#[cfg(not(target_arch = "wasm32"))]
pub fn headless_device() -> Result<(wgpu::Device, wgpu::Queue), Error> {
	pollster::block_on(async {
		let instance =
			wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = instance
			.request_adapter(&wgpu::RequestAdapterOptions::default())
			.await
			.map_err(|error| Error::NoAdapter(error.to_string()))?;
		adapter
			.request_device(&wgpu::DeviceDescriptor {
				label: Some("leptoskit_canvas"),
				required_limits: wgpu::Limits::downlevel_defaults()
					.using_resolution(adapter.limits()),
				..wgpu::DeviceDescriptor::default()
			})
			.await
			.map_err(|error| Error::Device(error.to_string()))
	})
}

/// Draws `scene` on a GPU device of its own and returns the pixels as RGBA
/// bytes, four per pixel, row by row, with premultiplied alpha. `width` and
/// `height` are in pixels and `scale` is how many pixels make one point.
///
/// It creates a device every call, so it suits tests and one-off images;
/// keep a [`Renderer`] to draw repeatedly.
#[cfg(not(target_arch = "wasm32"))]
pub fn render_to_rgba(
	scene: &Scene,
	width: u32,
	height: u32,
	scale: f32,
) -> Result<Vec<u8>, Error> {
	let (device, queue) = headless_device()?;
	Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm)
		.render_to_rgba(scene, width, height, scale)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn errors_describe_themselves_and_availability() {
		assert_eq!(
			Error::NoAdapter("none".into()).to_string(),
			"no GPU adapter is available: none"
		);
		assert_eq!(
			Error::Device("denied".into()).to_string(),
			"the GPU device could not be created: denied"
		);
		assert_eq!(
			Error::TooLarge {
				width: 100,
				height: 1,
				limit: 99
			}
			.to_string(),
			"the canvas is 100×1 pixels, larger than the GPU limit of 99"
		);
		assert_eq!(
			Error::UnsupportedFormat(wgpu::TextureFormat::Rg16Float).to_string(),
			"pixels cannot be read back from the Rg16Float format"
		);
		assert_eq!(
			Error::Readback("lost".into()).to_string(),
			"reading the pixels back failed: lost"
		);
		assert!(Error::NoAdapter("none".into()).is_unavailable());
		assert!(Error::Device("denied".into()).is_unavailable());
		assert!(
			!Error::TooLarge {
				width: 9,
				height: 9,
				limit: 8
			}
			.is_unavailable()
		);
		assert!(!Error::UnsupportedFormat(wgpu::TextureFormat::Rg16Float).is_unavailable());
		assert!(!Error::Readback("lost".into()).is_unavailable());
	}
}
