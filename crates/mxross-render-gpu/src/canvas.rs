// crates/mxross-render-gpu/src/canvas.rs
//! The actual paintable surface — a flat textured quad in world space.
//! Painting is stamping circular dabs directly into its texture via a
//! GPU render pass (`LoadOp::Load`, so existing paint persists).
//!
//! The canvas texture itself always stores true alpha (transparent
//! where unpainted) — that never changes, since it's what makes export
//! possible at all. `BackgroundMode` is purely a *presentation* setting:
//! it controls what the display pipeline composites the true-alpha
//! canvas against (a checkerboard — the universal "this is actually
//! transparent" convention every painting app uses, not a literal void —
//! or a solid color), and the caller is expected to use that same mode
//! to decide whether export should flatten onto a color
//! (`mxross_export::flatten_onto`) or keep transparency as-is. The
//! underlying pixel data is identical either way.

use wgpu::util::DeviceExt;

use mxross_math::Mat4;

/// Half-extent of the *longer* edge of the canvas plane in world units,
/// at a 1:1 (square) aspect ratio. Actual per-canvas half-extents are
/// computed from this plus the canvas's real aspect ratio so a
/// landscape or portrait canvas gets a proportionally-shaped plane
/// instead of always being forced square in world space — see
/// `half_extents_for`.
const CANVAS_BASE_HALF_SIZE: f32 = 2.0;

/// Given a canvas resolution, returns `(half_width, half_height)` in
/// world units: the longer edge is always `CANVAS_BASE_HALF_SIZE`, the
/// shorter edge scaled down to match the true aspect ratio. Keeps a
/// 1920x1080 canvas looking like a landscape rectangle in world space
/// rather than a square with letterboxing.
fn half_extents_for(width_px: u32, height_px: u32) -> (f32, f32) {
    let aspect = width_px as f32 / height_px as f32;
    if aspect >= 1.0 {
        (CANVAS_BASE_HALF_SIZE, CANVAS_BASE_HALF_SIZE / aspect)
    } else {
        (CANVAS_BASE_HALF_SIZE * aspect, CANVAS_BASE_HALF_SIZE)
    }
}

/// One dab to draw: where (canvas UV, 0..1, top-left origin), how big
/// (canvas texture pixels), what color. Plain data — no brush concepts
/// attached. `mxross-brush`'s `DabPlan` has the same shape; the two
/// types stay separate on purpose (see crate doc comment).
#[derive(Clone, Copy)]
pub struct Dab {
    pub position: (f32, f32),
    pub radius_px: f32,
    pub color: [f32; 4],
}

/// What the display pipeline composites the canvas against. Doesn't
/// touch the underlying canvas data at all — see module doc comment.
#[derive(Clone, Copy, PartialEq)]
pub enum BackgroundMode {
    /// Shown as a checkerboard, the standard "actually transparent, not
    /// a dark void" indicator.
    Transparent,
    Solid([f32; 3]),
}

impl BackgroundMode {
    pub fn white() -> Self {
        BackgroundMode::Solid([1.0, 1.0, 1.0])
    }

    fn as_uniform(self) -> [f32; 4] {
        match self {
            BackgroundMode::Transparent => [0.0, 0.0, 0.0, 0.0],
            BackgroundMode::Solid([r, g, b]) => [r, g, b, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct DisplayVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct StampVertex {
    position: [f32; 2], // NDC, in canvas-texture space
    local: [f32; 2],    // -1..1, for the circle falloff test
    color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

/// `.rgb` = solid color (ignored in transparent mode), `.a` = mode flag
/// (0.0 = transparent/checkerboard, 1.0 = solid) — packed as one vec4 to
/// avoid any WGSL uniform-struct alignment/padding questions entirely.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct BackgroundUniform {
    data: [f32; 4],
}

/// Erasing reuses the exact same dab shape/falloff shader as painting
/// (`canvas_stamp.wgsl` — see its `fs_main` doc comment) — only the
/// blend state differs. `src_factor: Zero` on both channels means the
/// dab's own color is never written; `dst_factor: OneMinusSrcAlpha`
/// scales down whatever's already on the canvas by `(1 - dab_alpha)` at
/// every pixel the dab covers. Since `dab_alpha` already carries the
/// shader's circular falloff, a dab erases with the same soft edge it
/// would paint with, instead of punching a hard-edged stencil hole the
/// way `BlendState::REPLACE` with a transparent color would.
const ERASE_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

pub struct PaintCanvas {
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    width_px: u32,
    height_px: u32,
    half_width: f32,
    half_height: f32,
    pixel_art: bool,
    display_pipeline: wgpu::RenderPipeline,
    display_vertex_buffer: wgpu::Buffer,
    display_index_buffer: wgpu::Buffer,
    display_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    background_buffer: wgpu::Buffer,
    stamp_pipeline: wgpu::RenderPipeline,
    erase_pipeline: wgpu::RenderPipeline,
}

impl PaintCanvas {
    /// `width_px`/`height_px` need not be square — see `half_extents_for`
    /// for how a non-square resolution shapes the world-space plane.
    ///
    /// `pixel_art` is a one-time choice baked in at construction, not a
    /// runtime toggle: it picks both the display sampler's filter mode
    /// (Nearest, so magnified pixels stay crisp squares instead of
    /// blending) and which stamp shader gets compiled in (hard-edged
    /// `canvas_stamp_pixel.wgsl` vs the default soft
    /// `canvas_stamp.wgsl`). Both live on the GPU pipeline/sampler
    /// objects themselves, which is why this can't be flipped after the
    /// fact without rebuilding the canvas — same as picking a canvas
    /// resolution.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        width_px: u32,
        height_px: u32,
        pixel_art: bool,
    ) -> Self {
        let (half_width, half_height) = half_extents_for(width_px, height_px);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("paint canvas texture"),
            size: wgpu::Extent3d { width: width_px, height: height_px, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Fully transparent to start — true alpha, never changes
        // regardless of BackgroundMode. See struct doc comment.
        let mut clear_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("paint canvas clear encoder"),
        });
        {
            let _pass = clear_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("paint canvas clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        queue.submit(std::iter::once(clear_encoder.finish()));

        // Nearest for pixel art: magnifying the canvas while zoomed in
        // shows actual pixel squares instead of the GPU blending
        // neighboring texels together. Linear (the previous, only,
        // behavior) is what a painting canvas normally wants — smooth
        // brush edges stay smooth when you zoom in on them.
        let filter_mode = if pixel_art { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("paint canvas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter_mode,
            min_filter: filter_mode,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("paint canvas camera buffer"),
            contents: bytemuck::cast_slice(&[CameraUniform {
                view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let background_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("paint canvas background buffer"),
            contents: bytemuck::cast_slice(&[BackgroundUniform {
                data: BackgroundMode::Transparent.as_uniform(),
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let display_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("paint canvas display bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let display_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("paint canvas display bind group"),
            layout: &display_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&texture_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: background_buffer.as_entire_binding() },
            ],
        });

        let display_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paint canvas display pipeline layout"),
            bind_group_layouts: &[Some(&display_bind_group_layout)],
            immediate_size: 0,
        });

        let display_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint canvas display shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/canvas_display.wgsl").into()),
        });

        let display_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<DisplayVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
            ],
        };

        let display_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("paint canvas display pipeline"),
            layout: Some(&display_layout),
            vertex: wgpu::VertexState {
                module: &display_shader,
                entry_point: Some("vs_main"),
                buffers: &[display_vertex_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &display_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // REPLACE, not ALPHA_BLENDING — the shader now does
                    // its own full compositing (canvas over checkerboard
                    // or canvas over solid color) and always outputs
                    // alpha 1.0, so there's nothing left for the GPU
                    // blend unit to do.
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let vertices = [
            DisplayVertex { position: [-half_width,  half_height, 0.0], uv: [0.0, 0.0] }, // top-left
            DisplayVertex { position: [ half_width,  half_height, 0.0], uv: [1.0, 0.0] }, // top-right
            DisplayVertex { position: [ half_width, -half_height, 0.0], uv: [1.0, 1.0] }, // bottom-right
            DisplayVertex { position: [-half_width, -half_height, 0.0], uv: [0.0, 1.0] }, // bottom-left
        ];
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

        let display_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("paint canvas display vertex buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let display_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("paint canvas display index buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let stamp_shader_source = if pixel_art {
            include_str!("shaders/canvas_stamp_pixel.wgsl")
        } else {
            include_str!("shaders/canvas_stamp.wgsl")
        };
        let stamp_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint canvas stamp shader"),
            source: wgpu::ShaderSource::Wgsl(stamp_shader_source.into()),
        });

        let stamp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paint canvas stamp pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        // Same shader, same vertex layout, same "no depth" targeting for
        // both paint and erase — only the blend state differs, so both
        // pipelines come from this one helper.
        let stamp_pipeline = Self::create_stamp_pipeline(
            device,
            &stamp_shader,
            &stamp_layout,
            wgpu::BlendState::ALPHA_BLENDING,
            "paint canvas stamp pipeline",
        );
        let erase_pipeline = Self::create_stamp_pipeline(
            device,
            &stamp_shader,
            &stamp_layout,
            ERASE_BLEND,
            "paint canvas erase pipeline",
        );

        Self {
            texture,
            texture_view,
            width_px,
            height_px,
            half_width,
            half_height,
            pixel_art,
            display_pipeline,
            display_vertex_buffer,
            display_index_buffer,
            display_bind_group,
            camera_buffer,
            background_buffer,
            stamp_pipeline,
            erase_pipeline,
        }
    }

    /// Shared by both stamp pipelines (paint + erase). No depth_stencil
    /// here — the stamp pass targets the canvas texture directly, which
    /// has no depth attachment at all.
    fn create_stamp_pipeline(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        layout: &wgpu::PipelineLayout,
        blend: wgpu::BlendState,
        label: &str,
    ) -> wgpu::RenderPipeline {
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<StampVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 2 },
            ],
        };

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn set_camera(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform { view_proj: view_proj.to_cols_array_2d() }]),
        );
    }

    /// Changes what the display pipeline composites the (always
    /// true-alpha) canvas texture against. Doesn't touch the canvas's
    /// own pixel data at all.
    pub fn set_background_mode(&self, queue: &wgpu::Queue, mode: BackgroundMode) {
        queue.write_buffer(
            &self.background_buffer,
            0,
            bytemuck::cast_slice(&[BackgroundUniform { data: mode.as_uniform() }]),
        );
    }

    /// Half-extents `(half_width, half_height)` of the canvas plane in
    /// world units — exposed so the caller can convert a touch position
    /// into canvas UV. Not necessarily square — see `half_extents_for`.
    pub fn half_extents(&self) -> (f32, f32) {
        (self.half_width, self.half_height)
    }

    /// Texture resolution in pixels as `(width, height)` — exposed so
    /// callers can convert UV-space distances into canvas-pixel
    /// distances. Needed by mxross-brush's spacing calculation, which
    /// works in real pixels. Not necessarily square.
    pub fn texture_size_px(&self) -> (f32, f32) {
        (self.width_px as f32, self.height_px as f32)
    }

    /// Whether this canvas was created in pixel-art mode — read back by
    /// `GpuState::snapshot_canvas` so a background/resume rebuilds the
    /// canvas with the same sampler/stamp-shader choice rather than
    /// silently reverting to smooth mode.
    pub fn is_pixel_art(&self) -> bool {
        self.pixel_art
    }

    /// Stamps every dab in `dabs` in one render pass — one encoder, one
    /// vertex/index buffer covering the whole batch, one submit.
    pub fn stamp_many(&self, device: &wgpu::Device, queue: &wgpu::Queue, dabs: &[Dab]) {
        self.stamp_with_pipeline(device, queue, dabs, &self.stamp_pipeline);
    }

    /// Same batching strategy as `stamp_many`, routed through
    /// `erase_pipeline` instead — see its doc comment / `ERASE_BLEND`
    /// for why this is the same dab shape with a different blend state
    /// rather than a separate code path. `dab.color`'s alpha still
    /// matters (it's the erase strength); its rgb doesn't, since
    /// `ERASE_BLEND` never writes source color.
    pub fn erase_many(&self, device: &wgpu::Device, queue: &wgpu::Queue, dabs: &[Dab]) {
        self.stamp_with_pipeline(device, queue, dabs, &self.erase_pipeline);
    }

    fn stamp_with_pipeline(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dabs: &[Dab],
        pipeline: &wgpu::RenderPipeline,
    ) {
        if dabs.is_empty() {
            return;
        }

        let mut vertices = Vec::with_capacity(dabs.len() * 4);
        let mut indices = Vec::with_capacity(dabs.len() * 6);

        for dab in dabs {
            let base = vertices.len() as u16;
            let center_ndc = [dab.position.0 * 2.0 - 1.0, 1.0 - dab.position.1 * 2.0];
            // Stamping happens directly in the texture's own NDC space
            // (-1..1 over the whole texture, regardless of its pixel
            // aspect ratio) — a single NDC radius would stretch a dab
            // into an ellipse the moment width_px != height_px, since
            // the same NDC delta covers a different pixel count on each
            // axis. Scaling x and y separately keeps a dab's real-pixel
            // footprint circular no matter the canvas's aspect ratio.
            let radius_ndc_x = dab.radius_px / self.width_px as f32 * 2.0;
            let radius_ndc_y = dab.radius_px / self.height_px as f32 * 2.0;

            for &(lx, ly) in &[(-1.0_f32, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                vertices.push(StampVertex {
                    position: [center_ndc[0] + lx * radius_ndc_x, center_ndc[1] + ly * radius_ndc_y],
                    local: [lx, ly],
                    color: dab.color,
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("brush stamp batch vertex buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("brush stamp batch index buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("brush stamp batch encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("brush stamp batch pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }
        queue.submit(std::iter::once(encoder.finish()));
    }

    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.display_pipeline);
        pass.set_bind_group(0, &self.display_bind_group, &[]);
        pass.set_vertex_buffer(0, self.display_vertex_buffer.slice(..));
        pass.set_index_buffer(self.display_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..1);
    }

    /// Reads the canvas texture back to CPU memory as tightly-packed
    /// RGBA8 (true alpha, regardless of the current `BackgroundMode` —
    /// that's purely a display-time setting; this always returns the
    /// real data). Returns `(width, height, pixels)`.
    ///
    /// Blocks the calling thread until the copy completes — this is a
    /// rare, user-triggered action (export), not something called every
    /// frame, so a blocking wait is the right tradeoff over async
    /// plumbing here.
    pub fn read_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> (u32, u32, Vec<u8>) {
        const BYTES_PER_PIXEL: u32 = 4; // Rgba8Unorm
        let (width, height) = (self.width_px, self.height_px);

        let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("paint canvas readback buffer"),
            size: (padded_bytes_per_row * height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("paint canvas readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });

        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .expect("device poll failed during canvas readback");
        rx.recv()
            .expect("map_async callback never fired")
            .expect("buffer mapping failed");

        let mapped = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            pixels.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        output_buffer.unmap();

        (width, height, pixels)
    }

    /// Inverse of `read_pixels`: uploads tightly-packed RGBA8 straight
    /// into the canvas texture. `queue.write_texture` does its own
    /// internal staging, so unlike `read_pixels` no 256-byte row
    /// alignment handling is needed on this side.
    ///
    /// No-op (rather than panicking) if `pixels` isn't exactly
    /// `width_px * height_px * 4` bytes for THIS canvas — a snapshot
    /// taken at a resolution that doesn't match the current canvas
    /// (e.g. after a "New Canvas" at a different size) should just be
    /// dropped, not crash the app on resume.
    pub fn write_pixels(&self, queue: &wgpu::Queue, pixels: &[u8]) {
        const BYTES_PER_PIXEL: u32 = 4; // Rgba8Unorm
        let expected = (self.width_px * self.height_px * BYTES_PER_PIXEL) as usize;
        if pixels.len() != expected {
            return;
        }

        queue.write_texture(
            self.texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width_px * BYTES_PER_PIXEL),
                rows_per_image: Some(self.height_px),
            },
            wgpu::Extent3d { width: self.width_px, height: self.height_px, depth_or_array_layers: 1 },
        );
    }
  }
