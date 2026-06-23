// crates/mxross-android/src/gpu.rs
//! wgpu GPU render path for MxRoss Canvas on Android — Instance -> Surface
//! -> Adapter -> Device -> Queue, then one depth-tested render pass per
//! frame drawing the test cube (test_cube.rs) from the camera (camera.rs)
//! followed by the egui UI (ui.rs) drawn flat on top.
//!
//! ## The HasDisplayHandle gap
//!
//! `ndk::native_window::NativeWindow` (0.9, default `rwh_06` feature)
//! implements `raw_window_handle::HasWindowHandle` but NOT
//! `HasDisplayHandle` — Android has no separate "display connection"
//! object the way X11/Wayland do, so `raw-window-handle` models it as
//! `AndroidDisplayHandle {}`, an explicitly empty marker struct, with a
//! `DisplayHandle::android()` constructor. `WindowHandles` below bundles
//! a `NativeWindow` with that marker so the combined type satisfies
//! wgpu's `DisplayAndWindowHandle` bound directly.

use ndk::native_window::NativeWindow;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::camera::OrbitCamera;
use crate::test_cube::TestCube;
use crate::ui::AppUi;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Bundles a `NativeWindow` with the (empty) Android display handle.
/// Takes ownership of the window — wgpu's `Surface` keeps this alive
/// internally for as long as the surface itself lives.
struct WindowHandles {
    native_window: NativeWindow,
}

impl HasWindowHandle for WindowHandles {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.native_window.window_handle()
    }
}

impl HasDisplayHandle for WindowHandles {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::android())
    }
}

/// Everything needed to render a frame. Rebuilt from scratch on every
/// Android `InitWindow` event and torn down on `TerminateWindow` (see
/// lib.rs) — which means camera mode and egui's internal state also
/// reset on a window swap (e.g. a real backgrounding/foregrounding
/// event). Acceptable for now; revisit only if that turns out to bite.
pub struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    test_cube: TestCube,
    camera: OrbitCamera,
    ui: AppUi,
    egui_renderer: egui_wgpu::Renderer,
    pending_ui_events: Vec<egui::Event>,
    /// Last single-finger touch position, in window pixel coordinates.
    /// None whenever no finger is down — cleared on touch_up so a fresh
    /// touch-down doesn't compute a delta against stale data.
    last_touch: Option<(f32, f32)>,
}

impl GpuState {
    /// Builds the full Instance -> Surface -> Adapter -> Device chain for
    /// `window`, configures the surface, and creates the depth buffer,
    /// test cube, and egui renderer at its current size.
    pub fn new(window: NativeWindow) -> Result<Self, String> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: NativeWindow) -> Result<Self, String> {
        let width = window.width().max(1) as u32;
        let height = window.height().max(1) as u32;

        // VULKAN explicitly, not Backends::all() — GLES is the
        // emulator/x86 fallback tier here, not something to design around;
        // the target device (Samsung A13) supports Vulkan directly.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: Default::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        let handles = WindowHandles { native_window: window };
        let surface: wgpu::Surface<'static> = instance
            .create_surface(handles)
            .map_err(|e| format!("failed to create wgpu surface: {e}"))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| format!("no suitable GPU adapter: {e}"))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mxross-android device"),
                required_features: wgpu::Features::empty(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| format!("failed to request GPU device: {e}"))?;

        let config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| "surface is not supported by this adapter".to_string())?;
        surface.configure(&device, &config);

        let depth_view = Self::create_depth_view(&device, width, height);
        let test_cube = TestCube::new(&device, config.format, DEPTH_FORMAT);
        // egui draws flat on top of the cube, ignoring depth entirely —
        // depth_stencil_format: None in RendererOptions::default() is
        // exactly that, not just a convenient default.
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, egui_wgpu::RendererOptions::default());

        Ok(Self {
            surface,
            device,
            queue,
            config,
            depth_view,
            test_cube,
            camera: OrbitCamera::new(),
            ui: AppUi::new(),
            egui_renderer,
            pending_ui_events: Vec::new(),
            last_touch: None,
        })
    }

    fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mxross-android depth texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Re-applies the config and rebuilds the depth buffer at a new size.
    /// Wired up to Android's `WindowResized` event (see lib.rs).
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = Self::create_depth_view(&self.device, width, height);
    }

    /// `x`/`y` are raw window pixel coordinates — converted to egui
    /// "points" internally using `pixels_per_point`.
    pub fn touch_down(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        self.last_touch = Some((x, y));
    }

    pub fn touch_move(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerMoved(egui::pos2(
            x / pixels_per_point,
            y / pixels_per_point,
        )));

        if let Some((lx, ly)) = self.last_touch {
            // Only orbit if egui didn't claim the pointer last frame —
            // see AppUi::pointer_over_ui's doc comment.
            if !self.ui.pointer_over_ui() {
                self.camera.handle_drag(x - lx, y - ly);
            }
        }
        self.last_touch = Some((x, y));
    }

    pub fn touch_up(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        self.pending_ui_events.push(egui::Event::PointerGone);
        self.last_touch = None;
    }

    /// Clears to `clear_color`, depth-tests the cube, then draws the
    /// egui UI flat on top.
    ///
    /// `Outdated`/`Lost` are deliberately just skipped for now rather
    /// than recovered from — fine while iterating, since InitWindow
    /// already rebuilds GpuState from scratch on any real window swap.
    pub fn render(&mut self, clear_color: wgpu::Color, pixels_per_point: f32) {
        let aspect = self.config.width as f32 / self.config.height as f32;
        self.test_cube.set_camera(&self.queue, self.camera.view_proj(aspect));

        let events = std::mem::take(&mut self.pending_ui_events);
        let output = self.ui.run_frame(
            &mut self.camera,
            events,
            (self.config.width, self.config.height),
            pixels_per_point,
        );
        let paint_jobs = self.ui.ctx().tessellate(output.shapes, output.pixels_per_point);
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mxross-android frame encoder"),
            });

        // Apply texture changes before painting, per egui's documented
        // contract (the font atlas texture shows up here on frame one).
        for (id, image_delta) in &output.textures_delta.set {
            self.egui_renderer.update_texture(&self.device, &self.queue, *id, image_delta);
        }

        let extra_cmds = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mxross-android frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            self.test_cube.draw(&mut pass);

            // egui-wgpu's render() requires a 'static render pass —
            // forget_lifetime() only relaxes a compile-time
            // "don't touch the encoder" check to a runtime one; see its
            // doc comment. Nothing unsafe, just a borrow-checker downgrade.
            let mut pass = pass.forget_lifetime();
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen_descriptor);
        }

        self.queue.submit(extra_cmds.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();

        // Free after painting, per egui's contract.
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
            }
