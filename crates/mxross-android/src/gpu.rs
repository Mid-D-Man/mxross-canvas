// crates/mxross-android/src/gpu.rs
//! wgpu GPU render path for MxRoss Canvas on Android — Instance -> Surface
//! -> Adapter -> Device -> Queue, then one depth-tested render pass per
//! frame drawing the paint canvas (mxross-render-gpu), with all pointer
//! routing (orbit/zoom/paint) handled by mxross-interaction's
//! `CanvasController`, and the egui UI (ui.rs) drawn flat on top.
//!
//! This is the one file allowed to know about `mxross-brush` (for
//! `DabPlan`), `mxross-render-gpu` (for `Dab`/`BackgroundMode`), and
//! `mxross-export` (for PNG encoding) all at once — that's deliberate.
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
//!
//! ## The egui depth_stencil gap
//!
//! A render pipeline with `depth_stencil: None` is NOT compatible with a
//! render pass that has a depth/stencil attachment (see `emilk/egui#2083`).
//! `Some(DEPTH_FORMAT)` below makes egui's pipeline declare a depth state
//! with `depth_write_enabled: false` / `depth_compare: Always`, staying
//! compatible with the pass without actually being depth-tested.

use ndk::native_window::NativeWindow;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use mxross_brush::DabPlan;
use mxross_interaction::CanvasController;
use mxross_render_gpu::{BackgroundMode, Dab, PaintCanvas};

use crate::ui::AppUi;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

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

pub struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    canvas: PaintCanvas,
    controller: CanvasController,
    background_mode: BackgroundMode,
    ui: AppUi,
    egui_renderer: egui_wgpu::Renderer,
    pending_ui_events: Vec<egui::Event>,
    pending_export: Option<Vec<u8>>,
}

impl GpuState {
    pub fn new(window: NativeWindow) -> Result<Self, String> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: NativeWindow) -> Result<Self, String> {
        let width = window.width().max(1) as u32;
        let height = window.height().max(1) as u32;

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
        let canvas = PaintCanvas::new(&device, &queue, config.format, DEPTH_FORMAT);
        let controller = CanvasController::new(
            canvas.half_size(),
            canvas.texture_size_px(),
            (width as f32, height as f32),
        );
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: Some(DEPTH_FORMAT),
                ..Default::default()
            },
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            depth_view,
            canvas,
            controller,
            background_mode: BackgroundMode::Transparent,
            ui: AppUi::new(),
            egui_renderer,
            pending_ui_events: Vec::new(),
            pending_export: None,
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

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = Self::create_depth_view(&self.device, width, height);
        self.controller.resize(width as f32, height as f32);
    }

    fn apply_dabs(&self, plans: Vec<DabPlan>) {
        if plans.is_empty() {
            return;
        }
        let dabs: Vec<Dab> = plans
            .into_iter()
            .map(|p| Dab { position: p.position, radius_px: p.radius_px, color: p.color })
            .collect();
        self.canvas.stamp_many(&self.device, &self.queue, &dabs);
    }

    pub fn touch_down(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let plans = self.controller.pointer_down(x, y, self.ui.pointer_over_ui());
        self.apply_dabs(plans);
    }

    pub fn second_touch_down(&mut self) {
        self.controller.second_pointer_down();
    }

    pub fn touch_move(&mut self, pointers: &[(f32, f32)], pixels_per_point: f32) {
        if let Some(&(x, y)) = pointers.first() {
            self.pending_ui_events.push(egui::Event::PointerMoved(egui::pos2(
                x / pixels_per_point,
                y / pixels_per_point,
            )));
        }
        let plans = self.controller.pointer_moved(pointers, self.ui.pointer_over_ui());
        self.apply_dabs(plans);
    }

    pub fn touch_up(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        self.pending_ui_events.push(egui::Event::PointerGone);
        let plans = self.controller.pointer_up(x, y);
        self.apply_dabs(plans);
    }

    fn export_png(&self) -> Result<Vec<u8>, String> {
        let (width, height, rgba) = self.canvas.read_pixels(&self.device, &self.queue);
        let pixels = match self.background_mode {
            BackgroundMode::Transparent => rgba,
            BackgroundMode::Solid([r, g, b]) => {
                let bg = [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8];
                mxross_export::flatten_onto(&rgba, bg)
            }
        };
        mxross_export::encode_png(width, height, &pixels)
    }

    pub fn take_pending_export(&mut self) -> Option<Vec<u8>> {
        self.pending_export.take()
    }

    /// Called by lib.rs right after it's tried (and either succeeded or
    /// failed) to write a pending export to disk — the actual,
    /// real-or-not confirmation you can see without logcat.
    pub fn set_export_status(&mut self, status: String) {
        self.ui.set_export_status(status);
    }

    pub fn render(&mut self, clear_color: wgpu::Color, pixels_per_point: f32) {
        // Commits any touch-start that's cleared the pinch-disambiguation
        // window since the last frame — see CanvasController::tick's doc
        // comment. Done before anything else so a freshly-committed dab
        // still shows up in THIS frame's draw, not the next one.
        let tick_dabs = self.controller.tick();
        self.apply_dabs(tick_dabs);
        let aspect = self.config.width as f32 / self.config.height as f32;
        self.canvas.set_camera(&self.queue, self.controller.camera().view_proj(aspect));

        let events = std::mem::take(&mut self.pending_ui_events);
        let output = self.ui.run_frame(
            self.controller.camera_mut(),
            self.background_mode,
            events,
            (self.config.width, self.config.height),
            pixels_per_point,
        );

        if self.ui.take_background_toggle_requested() {
            self.background_mode = match self.background_mode {
                BackgroundMode::Transparent => BackgroundMode::white(),
                BackgroundMode::Solid(_) => BackgroundMode::Transparent,
            };
            self.canvas.set_background_mode(&self.queue, self.background_mode);
        }

        if self.ui.take_export_request() {
            match self.export_png() {
                Ok(bytes) => self.pending_export = Some(bytes),
                Err(e) => {
                    log::error!("PNG export failed: {e}");
                    self.ui.set_export_status(format!("Export failed: {e}"));
                }
            }
        }

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

            self.canvas.draw(&mut pass);

            let mut pass = pass.forget_lifetime();
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen_descriptor);
        }

        self.queue.submit(extra_cmds.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();

        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
                                 }
