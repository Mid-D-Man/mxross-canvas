// crates/mxross-render-gpu/src/canvas.rs
//! The actual paintable surface — a flat textured quad in world space.
//! Painting is stamping circular dabs directly into its texture via a
//! GPU render pass (`LoadOp::Load`, so existing paint persists).
//!
//! This struct does exactly one job: given a list of dabs (where, how
//! big, what color), draw them — plus, now, read the result back to CPU
//! memory for export. It has no concept of strokes, spacing, smoothing,
//! presets, or file formats — that's `mxross-brush` and `mxross-export`'s
//! jobs respectively.
//!
//! The canvas starts fully transparent (not white) — unpainted areas
//! have alpha 0, which matters for export (a drawing surface with an
//! opaque white background can't ever produce a transparent PNG, no
//! matter what the export step does) and, just as importantly, for how
//! it displays in the 3D scene: the display pipeline blends with
//! `ALPHA_BLENDING`, not `REPLACE`, specifically so transparent canvas
//! pixels let the scene's background show through instead of painting
//! solid black where nothing's been drawn.

use wgpu::util::DeviceExt;

use mxross_math::Mat4;

const TEXTURE_SIZE: u32 = 1024;
/// Half-extent of the canvas plane in world units — the plane spans
/// -CANVAS_HALF_SIZE..CANVAS_HALF_SIZE on both X and Y, at Z=0.
const CANVAS_HALF_SIZE: f32 = 2.0;

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

pub struct PaintCanvas {
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    display_pipeline: wgpu::RenderPipeline,
    display_vertex_buffer: wgpu::Buffer,
    display_index_buffer: wgpu::Buffer,
    display_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    stamp_pipeline: wgpu::RenderPipeline,
}

impl PaintCanvas {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("paint canvas texture"),
            size: wgpu::Extent3d { width: TEXTURE_SIZE, height: TEXTURE_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Fully transparent to start — NOT white. See struct doc comment
        // for why this matters both for export and for how the canvas
        // displays in-app.
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("paint canvas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
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
            ],
        });

        let display_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("paint canvas display bind group"),
            layout: &display_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&texture_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
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
                    // ALPHA_BLENDING, not REPLACE — see struct doc
                    // comment. Without this, a transparent canvas would
                    // render as solid black in-app wherever nothing's
                    // been painted, instead of showing the scene
                    // background through it.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

        let half = CANVAS_HALF_SIZE;
        let vertices = [
            DisplayVertex { position: [-half,  half, 0.0], uv: [0.0, 0.0] }, // top-left
            DisplayVertex { position: [ half,  half, 0.0], uv: [1.0, 0.0] }, // top-right
            DisplayVertex { position: [ half, -half, 0.0], uv: [1.0, 1.0] }, // bottom-right
            DisplayVertex { position: [-half, -half, 0.0], uv: [0.0, 1.0] }, // bottom-left
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

        let stamp_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint canvas stamp shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/canvas_stamp.wgsl").into()),
        });

        let stamp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paint canvas stamp pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let stamp_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<StampVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 2 },
            ],
        };

        // No depth_stencil here — the stamp pass targets the canvas
        // texture directly, which has no depth attachment at all.
        let stamp_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("paint canvas stamp pipeline"),
            layout: Some(&stamp_layout),
            vertex: wgpu::VertexState {
                module: &stamp_shader,
                entry_point: Some("vs_main"),
                buffers: &[stamp_vertex_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &stamp_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            texture,
            texture_view,
            display_pipeline,
            display_vertex_buffer,
            display_index_buffer,
            display_bind_group,
            camera_buffer,
            stamp_pipeline,
        }
    }

    pub fn set_camera(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform { view_proj: view_proj.to_cols_array_2d() }]),
        );
    }

    /// Half-extent of the canvas plane in world units — exposed so the
    /// caller can convert a touch position into canvas UV.
    pub fn half_size(&self) -> f32 {
        CANVAS_HALF_SIZE
    }

    /// Texture resolution in pixels (square) — exposed so callers can
    /// convert UV-space distances into canvas-pixel distances. Needed by
    /// mxross-brush's spacing calculation, which works in real pixels
    /// (matching Krita/Photoshop's definition of "spacing") rather than
    /// abstract 0..1 UV fractions.
    pub fn texture_size_px(&self) -> f32 {
        TEXTURE_SIZE as f32
    }

    /// Stamps every dab in `dabs` in one render pass — one encoder, one
    /// vertex/index buffer covering the whole batch, one submit.
    pub fn stamp_many(&self, device: &wgpu::Device, queue: &wgpu::Queue, dabs: &[Dab]) {
        if dabs.is_empty() {
            return;
        }

        let mut vertices = Vec::with_capacity(dabs.len() * 4);
        let mut indices = Vec::with_capacity(dabs.len() * 6);

        for dab in dabs {
            let base = vertices.len() as u16;
            let center_ndc = [dab.position.0 * 2.0 - 1.0, 1.0 - dab.position.1 * 2.0];
            let radius_ndc = dab.radius_px / TEXTURE_SIZE as f32 * 2.0;

            for &(lx, ly) in &[(-1.0_f32, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                vertices.push(StampVertex {
                    position: [center_ndc[0] + lx * radius_ndc, center_ndc[1] + ly * radius_ndc],
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
            pass.set_pipeline(&self.stamp_pipeline);
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
    /// RGBA8 (no row padding in the result, even though the GPU copy
    /// itself requires 256-byte-aligned rows — that padding is stripped
    /// here so callers never need to know it exists). Returns
    /// `(width, height, pixels)`.
    ///
    /// Blocks the calling thread until the copy completes — this is a
    /// rare, user-triggered action (export), not something called every
    /// frame, so a blocking wait is the right tradeoff over async
    /// plumbing here.
    pub fn read_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> (u32, u32, Vec<u8>) {
        const BYTES_PER_PIXEL: u32 = 4; // Rgba8Unorm

        let unpadded_bytes_per_row = TEXTURE_SIZE * BYTES_PER_PIXEL;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("paint canvas readback buffer"),
            size: (padded_bytes_per_row * TEXTURE_SIZE) as wgpu::BufferAddress,
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
                    rows_per_image: Some(TEXTURE_SIZE),
                },
            },
            wgpu::Extent3d { width: TEXTURE_SIZE, height: TEXTURE_SIZE, depth_or_array_layers: 1 },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });

        // Blocks until the copy (and the map_async callback above) have
        // both completed.
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .expect("device poll failed during canvas readback");
        rx.recv()
            .expect("map_async callback never fired")
            .expect("buffer mapping failed");

        let mapped = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * TEXTURE_SIZE) as usize);
        for row in 0..TEXTURE_SIZE {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            pixels.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        output_buffer.unmap();

        (TEXTURE_SIZE, TEXTURE_SIZE, pixels)
    }
}
