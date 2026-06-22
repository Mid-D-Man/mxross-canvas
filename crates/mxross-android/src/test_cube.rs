// crates/mxross-android/src/test_cube.rs
//! Hardcoded six-colored-face cube — a throwaway test primitive to prove
//! the GPU path actually renders real 3D geometry with correct depth, not
//! just a flat clear color. This is NOT the brush/canvas renderer; it
//! gets replaced once mid-math's camera lands and there's real scene
//! content to draw.
//!
//! Camera here is a fixed perspective view, not the locked-orthographic
//! free-roam viewport decided on for the real canvas — perspective makes
//! it visually obvious whether 3D depth/orientation is actually correct,
//! which a straight-on orthographic view of one face wouldn't prove.
//!
//! No backface culling: `primitive: PrimitiveState::default()` leaves
//! `cull_mode: None`, so the cube's vertex winding doesn't need to be
//! exactly right — the depth buffer alone decides what's visible.

use wgpu::util::DeviceExt;

use mxross_math::{Mat4, Vec3};

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

/// One face's four corners, in perimeter order. Winding doesn't matter —
/// see module doc comment on culling.
fn face(corners: [[f32; 3]; 4], color: [f32; 3]) -> [Vertex; 4] {
    [
        Vertex { position: corners[0], color },
        Vertex { position: corners[1], color },
        Vertex { position: corners[2], color },
        Vertex { position: corners[3], color },
    ]
}

fn cube_vertices() -> Vec<Vertex> {
    // Unit cube, half-extent 0.5, centered at the origin.
    let p = [
        [-0.5, -0.5, -0.5], // 0
        [ 0.5, -0.5, -0.5], // 1
        [ 0.5,  0.5, -0.5], // 2
        [-0.5,  0.5, -0.5], // 3
        [-0.5, -0.5,  0.5], // 4
        [ 0.5, -0.5,  0.5], // 5
        [ 0.5,  0.5,  0.5], // 6
        [-0.5,  0.5,  0.5], // 7
    ];

    let mut verts = Vec::with_capacity(24);
    verts.extend(face([p[0], p[3], p[7], p[4]], [0.0, 1.0, 1.0])); // -X cyan
    verts.extend(face([p[1], p[2], p[6], p[5]], [1.0, 0.0, 0.0])); // +X red
    verts.extend(face([p[0], p[1], p[5], p[4]], [1.0, 0.0, 1.0])); // -Y magenta
    verts.extend(face([p[3], p[2], p[6], p[7]], [0.0, 1.0, 0.0])); // +Y green
    verts.extend(face([p[0], p[1], p[2], p[3]], [1.0, 1.0, 0.0])); // -Z yellow
    verts.extend(face([p[4], p[5], p[6], p[7]], [0.0, 0.0, 1.0])); // +Z blue
    verts
}

fn cube_indices() -> Vec<u16> {
    let mut idx = Vec::with_capacity(36);
    for face_index in 0..6u16 {
        let base = face_index * 4;
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    idx
}

pub struct TestCube {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    camera_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl TestCube {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test cube shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/test_cube.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test cube camera bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("test cube pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test cube pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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

        let vertices = cube_vertices();
        let indices = cube_indices();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test cube vertex buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test cube index buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("test cube camera buffer"),
            contents: bytemuck::cast_slice(&[CameraUniform {
                view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test cube camera bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            camera_buffer,
            bind_group,
        }
    }

    /// Recomputes the view-projection matrix for the given aspect ratio
    /// and uploads it. Fixed eye position/target — there's no orbit
    /// control yet, this is purely "prove the depth/orientation math is
    /// right", not the real camera.
    pub fn update_camera(&self, queue: &wgpu::Queue, aspect: f32) {
        let eye = Vec3::new(2.0, 1.8, 2.5);
        let proj = Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let view_proj = proj * view;

        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform { view_proj: view_proj.to_cols_array_2d() }]),
        );
    }

    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
      }
