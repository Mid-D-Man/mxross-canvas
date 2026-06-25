// crates/mxross-android/src/gpu.rs
//! wgpu GPU render path for MxRoss Canvas on Android — Instance -> Surface
//! -> Adapter -> Device -> Queue, then one depth-tested render pass per
//! frame drawing the paint canvas (canvas.rs) from the camera (camera.rs)
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

use crate::brush::BrushSettings;
use crate::camera::{CameraMode, OrbitCamera};
use crate::canvas::{self, PaintCanvas};
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
/// lib.rs) — which means camera mode, the paint canvas, and egui's
/// internal state all reset on a window swap. Acceptable for now;
/// revisit (the canvas especially — losing a painting on backgrounding
/// would actually be bad) once there's a real save/load path.
pub struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    canvas: PaintCanvas,
    camera: OrbitCamera,
    brush: BrushSettings,
    ui: AppUi,
    egui_renderer: egui_wgpu::Renderer,
    pending_ui_events: Vec<egui::Event>,
    /// Last single-finger touch position, in window pixel coordinates.
    /// None whenever no finger is down.
    last_touch: Option<(f32, f32)>,
    /// Distance between the first two active pointers, in pixels, as of
    /// the last `touch_move` call. None whenever fewer than two fingers
    /// are down.
    last_pinch_distance: Option<f32>,
    /// True for the duration of a single-finger stroke that started
    /// somewhere paintable. Set on `touch_down`, cleared on `touch_up`
    /// and on `second_touch_down` — that second case matters: without
    /// it, a second finger landing mid-stroke (to start a pinch) would
    /// otherwise leave one stray dab at wherever the first finger
    /// happened to be, since `touch_move`'s two-pointer branch wouldn't
    /// have stopped it any other way.
    is_painting: bool,
}

impl GpuState {
    /// Builds the full Instance -> Surface -> Adapter -> Device chain for
    /// `window`, configures the surface, and creates the depth buffer,
    /// paint canvas, and egui renderer at its current size.
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
        let canvas = PaintCanvas::new(&device, &queue, config.format, DEPTH_FORMAT);
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
            camera: OrbitCamera::new(),
            brush: BrushSettings::default_ink(),
            ui: AppUi::new(),
            egui_renderer,
            pending_ui_events: Vec::new(),
            last_touch: None,
            last_pinch_distance: None,
            is_painting: false,
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

    fn can_paint(&self) -> bool {
        self.camera.mode() == CameraMode::LockedOrtho && self.camera.is_front_view()
    }

    fn try_stamp(&self, x: f32, y: f32) {
        let aspect = self.config.width as f32 / self.config.height as f32;
        let ortho_extents = self.camera.ortho_half_extents(aspect);
        if let Some(uv) = canvas::screen_to_canvas_uv(
            x,
            y,
            self.config.width as f32,
            self.config.height as f32,
            ortho_extents,
            self.canvas.half_size(),
        ) {
            self.canvas.stamp(&self.device, &self.queue, uv, &self.brush);
        }
    }

    /// `x`/`y` are raw window pixel coordinates — converted to egui
    /// "points" internally using `pixels_per_point`. Only for a genuine
    /// first finger (Android `MotionAction::Down`) — a second finger
    /// landing (`PointerDown`) goes through `second_touch_down` instead.
    pub fn touch_down(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        self.last_touch = Some((x, y));

        // One-frame-stale pointer_over_ui — same accepted caveat as
        // camera-drag arbitration: tapping directly on a UI element
        // could in principle leave one stray dab before egui catches up
        // the following frame. In practice the canvas doesn't reach the
        // screen corners where the UI lives at default zoom, so this is
        // a narrow edge case, not a constant annoyance.
        self.is_painting = self.can_paint() && !self.ui.pointer_over_ui();
        if self.is_painting {
            self.try_stamp(x, y);
        }
    }

    /// A second finger touched down — treated as the start of a pinch,
    /// not painting. Cancels any in-progress stroke so the pinch doesn't
    /// leave a stray dab where the first finger happened to be.
    pub fn second_touch_down(&mut self) {
        self.is_painting = false;
    }

    /// `pointers` are ALL currently active touches, in window pixel
    /// coordinates. egui only ever sees the first one (it has no concept
    /// of multi-touch); two pointers drive pinch-to-zoom instead of
    /// painting or camera orbit.
    pub fn touch_move(&mut self, pointers: &[(f32, f32)], pixels_per_point: f32) {
        let Some(&(x, y)) = pointers.first() else { return };

        self.pending_ui_events.push(egui::Event::PointerMoved(egui::pos2(
            x / pixels_per_point,
            y / pixels_per_point,
        )));

        if pointers.len() >= 2 {
            let (x1, y1) = pointers[1];
            let distance = ((x1 - x).powi(2) + (y1 - y).powi(2)).sqrt();
            if let Some(last) = self.last_pinch_distance {
                self.camera.zoom(distance / last.max(1.0));
            }
            self.last_pinch_distance = Some(distance);
            self.is_painting = false;
        } else {
            self.last_pinch_distance = None;
            if self.is_painting {
                self.try_stamp(x, y);
            } else if let Some((lx, ly)) = self.last_touch {
                // Only orbit if egui didn't claim the pointer last frame
                // — see AppUi::pointer_over_ui's doc comment. Harmless
                // either way in LockedOrtho (handle_drag no-ops there),
                // but skipping it while painting avoids calling it for
                // no reason every single stroke sample.
                if !self.ui.pointer_over_ui() {
                    self.camera.handle_drag(x - lx, y - ly);
                }
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
        self.last_pinch_distance = None;
        self.is_painting = false;
    }

    /// Clears to `clear_color`, depth-tests the canvas plane, then draws
    /// the egui UI flat on top.
    ///
    /// `Outdated`/`Lost` are deliberately just skipped for now rather
    /// than recovered from — fine while iterating, since InitWindow
    /// already rebuilds GpuState from scratch on any real window swap.
    pub fn render(&mut self, clear_color: wgpu::Color, pixels_per_point: f32) {
        let aspect = self.config.width as f32 / self.config.height as f32;
        self.canvas.set_camera(&self.queue, self.camera.view_proj(aspect));

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
