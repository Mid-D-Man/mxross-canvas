// crates/mxross-android/src/canvas.rs
//! The actual paintable surface — a flat textured quad in world space,
//! painted by stamping circular dabs directly into its texture via tiny
//! one-off GPU render passes (`LoadOp::Load`, so existing paint
//! persists). This is the `Surface2D` brush kind from brush.rs.
//!
//! Touch input only maps onto this plane correctly while the camera is
//! in its LockedOrtho front view (`OrbitCamera::is_front_view`) — the
//! mapping here is a simple orthographic unproject, not a general
//! ray-plane intersection. Painting from Top/Side/Back or in FreeOrbit
//! isn't wired up; that's a later generalization (and the natural
//! on-ramp toward `Mesh3D` brushes, once there's real geometry to ray-
//! cast against instead of one fixed flat plane).

use wgpu::util::DeviceExt;

use mxross_math::Mat4;

use crate::brush::BrushSettings;

const TEXTURE_SIZE: u32 = 1024;
/// Half-extent of the canvas plane in world units — the plane spans
/// -CANVAS_HALF_SIZE..CANVAS_HALF_SIZE on both X and Y, at Z=0.
const CANVAS_HALF_SIZE: f32 = 2.0;

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
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Blank white canvas to start — one clear-only pass, no draws.
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
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
        // texture directly, which has no depth attachment at all (unlike
        // the main scene pass).
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

    /// Half-extent of the canvas plane in world units — exposed so
    /// GpuState can convert a front-view touch position into canvas UV.
    pub fn half_size(&self) -> f32 {
        CANVAS_HALF_SIZE
    }

    /// Stamps one circular dab at `uv` (0..1, top-left origin). Does its
    /// own one-off encoder + submit, independent of the main frame's —
    /// simplest correct thing given stamps can arrive multiple times per
    /// frame (one per touch-move sample) and each needs its own vertex
    /// data; revisit only if per-stamp submit overhead actually shows up
    /// as a real cost on device.
    pub fn stamp(&self, device: &wgpu::Device, queue: &wgpu::Queue, uv: (f32, f32), brush: &BrushSettings) {
        let center_ndc = [uv.0 * 2.0 - 1.0, 1.0 - uv.1 * 2.0];
        let radius_ndc = [
            brush.radius_px / TEXTURE_SIZE as f32 * 2.0,
            brush.radius_px / TEXTURE_SIZE as f32 * 2.0,
        ];

        let corner = |lx: f32, ly: f32| StampVertex {
            position: [center_ndc[0] + lx * radius_ndc[0], center_ndc[1] + ly * radius_ndc[1]],
            local: [lx, ly],
            color: brush.color,
        };
        let vertices = [
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
            corner(1.0, -1.0),
            corner(-1.0, -1.0),
        ];
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("brush stamp vertex buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("brush stamp index buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("brush stamp encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("brush stamp pass"),
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
            pass.draw_indexed(0..6, 0, 0..1);
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
}

/// Converts a touch position (raw window pixels) into canvas UV
/// (0..1, top-left origin) — only valid while the camera is in its
/// locked-ortho front view (`OrbitCamera::is_front_view`); this is a
/// simple orthographic unproject, not a general ray-plane intersection.
/// Returns `None` if the touch lands outside the canvas plane.
pub fn screen_to_canvas_uv(
    x: f32,
    y: f32,
    screen_width: f32,
    screen_height: f32,
    ortho_half_extents: (f32, f32),
    canvas_half_size: f32,
) -> Option<(f32, f32)> {
    let ndc_x = (x / screen_width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (y / screen_height) * 2.0;

    let world_x = ndc_x * ortho_half_extents.0;
    let world_y = ndc_y * ortho_half_extents.1;

    let u = (world_x / canvas_half_size + 1.0) / 2.0;
    let v = (1.0 - world_y / canvas_half_size) / 2.0;

    if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
        Some((u, v))
    } else {
        None
    }
         }
