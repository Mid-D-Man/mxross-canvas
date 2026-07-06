// crates/mxross-android/src/gpu.rs
//! wgpu GPU render path for MxRoss Canvas on Android — Instance -> Surface
//! -> Adapter -> Device -> Queue, then one depth-tested render pass per
//! frame drawing either the "New Canvas" setup screen or the paint
//! canvas (mxross-render-gpu), with all pointer routing (orbit/zoom/
//! paint) handled by mxross-interaction's `CanvasController`, and the
//! egui UI (ui.rs) drawn flat on top either way.
//!
//! This is the one file allowed to know about `mxross-brush` (for
//! `DabPlan`), `mxross-render-gpu` (for `Dab`/`BackgroundMode`), and
//! `mxross-export` (for PNG encoding) all at once — that's deliberate.
//!
//! ## Screen state machine
//!
//! The app boots into `Screen::Home` — the entry tiles (New Canvas /
//! Continue / Gallery, the latter two disabled until real project
//! save/load exists). "New Canvas" moves to `Screen::Setup`: no
//! `PaintCanvas` exists yet, only the device/surface/depth target. Pick
//! a preset or a custom width/height there (ui.rs's `run_setup_frame`)
//! and `PaintCanvas`/`CanvasController` get constructed for the very
//! first time, transitioning into `Screen::Painting`. There's
//! deliberately no way back from `Screen::Painting` to an earlier screen
//! yet — see the note in the chat response this shipped with for why
//! that's scoped out rather than half-built.
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

/// Which GPU stamp pipeline dabs route through. Lives here rather than
/// in `mxross-brush` or `mxross-interaction` because nothing about a
/// stroke's shape, smoothing, or spacing changes between paint and erase
/// — only which blend state paints it onto the canvas texture. Both
/// tools share the same `BrushPreset` (radius/spacing), so the brush-size
/// slider affects whichever tool is currently selected.
#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    Paint,
    Erase,
}

impl Tool {
    fn toggled(self) -> Self {
        match self {
            Tool::Paint => Tool::Erase,
            Tool::Erase => Tool::Paint,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Paint => "Tool: Brush",
            Tool::Erase => "Tool: Eraser",
        }
    }
}

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

/// Everything that only exists once a canvas size has actually been
/// chosen. Absent in `Screen::Setup` — there's nothing to paint on, no
/// tool, no background mode, until "New Canvas" is confirmed.
struct PaintingState {
    canvas: PaintCanvas,
    controller: CanvasController,
    background_mode: BackgroundMode,
    tool: Tool,
}

enum Screen {
    /// The very first screen shown on launch — "New Canvas" leads to
    /// `Screen::Setup`; Continue/Gallery are visible but disabled (see
    /// `AppUi::run_home_frame`'s doc comment for why).
    Home,
    /// The entry screen: presets + custom width/height, gating
    /// everything else until a size is chosen. Carries no payload of its
    /// own — the actual typed-in width/height live in `AppUi` alongside
    /// its other one-shot widget state.
    Setup,
    Painting(PaintingState),
}

pub struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    /// Queried once at device creation (`device.limits().max_texture_dimension_2d`)
    /// and handed to the setup screen so its custom width/height inputs
    /// can't be dragged past what this GPU can actually allocate.
    max_texture_dimension: u32,
    screen: Screen,
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

        let max_texture_dimension = device.limits().max_texture_dimension_2d;

        let config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| "surface is not supported by this adapter".to_string())?;
        surface.configure(&device, &config);

        let depth_view = Self::create_depth_view(&device, width, height);
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
            max_texture_dimension,
            screen: Screen::Home,
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
        if let Screen::Painting(state) = &mut self.screen {
            state.controller.resize(width as f32, height as f32);
        }
    }

    /// Reads the canvas back to CPU-side pixels plus its resolution —
    /// called right before this whole `GpuState` (including the GPU
    /// texture holding the painting) is dropped on `TerminateWindow`.
    /// Android backgrounding tears down the native window, and this
    /// app's response to that is to fully tear down and later rebuild
    /// the wgpu Device/Surface/canvas from scratch (see lib.rs) rather
    /// than try to keep a GPU context alive across it — safe, but it
    /// means the painting itself needs to survive that boundary by some
    /// other route. This is that route: lib.rs holds onto the returned
    /// `(width, height, pixels)` outside `GpuState` and hands it to
    /// `restore_canvas` once the next `GpuState::new` succeeds.
    ///
    /// Returns `None` if backgrounding happened while still on the Setup
    /// screen — nothing painted yet, nothing to lose, so the app just
    /// resumes back into Setup.
    pub fn snapshot_canvas(&self) -> Option<(u32, u32, bool, Vec<u8>)> {
        match &self.screen {
            Screen::Painting(state) => {
                let (width, height, pixels) = state.canvas.read_pixels(&self.device, &self.queue);
                Some((width, height, state.canvas.is_pixel_art(), pixels))
            }
            Screen::Setup | Screen::Home => None,
        }
    }

    /// Rebuilds a `Screen::Painting` at the given resolution and pixel-
    /// art setting, and uploads a snapshot from `snapshot_canvas`
    /// straight into it — skips re-showing the Setup screen after a
    /// resume, going right back to where painting left off. `pixel_art`
    /// survives the round trip since it's read back from the canvas
    /// itself (see `PaintCanvas::is_pixel_art`), but background mode /
    /// tool / brush radius don't — those lived on the dropped
    /// `PaintingState`, not in the snapshot. A known, minor gap, not a
    /// silent one.
    pub fn restore_canvas(&mut self, width: u32, height: u32, pixel_art: bool, pixels: &[u8]) {
        let canvas = PaintCanvas::new(&self.device, &self.queue, self.config.format, DEPTH_FORMAT, width, height, pixel_art);
        canvas.write_pixels(&self.queue, pixels);
        let controller = CanvasController::new(
            canvas.half_extents(),
            canvas.texture_size_px(),
            (self.config.width as f32, self.config.height as f32),
        );
        self.screen = Screen::Painting(PaintingState {
            canvas,
            controller,
            background_mode: BackgroundMode::Transparent,
            tool: Tool::Paint,
        });
    }

    /// Free-standing on purpose (no `&self`) — the `Screen::Painting`
    /// match arm in `render`/`touch_*` already holds a mutable borrow of
    /// `self.screen`, so a helper that took `&self`/`&mut self` as a
    /// whole would conflict with it. Taking each piece explicitly keeps
    /// this a plain data operation the borrow checker has no objection
    /// to, called as `Self::apply_dabs(&self.device, &self.queue,
    /// &state.canvas, state.tool, plans)`.
    fn apply_dabs(device: &wgpu::Device, queue: &wgpu::Queue, canvas: &PaintCanvas, tool: Tool, plans: Vec<DabPlan>) {
        if plans.is_empty() {
            return;
        }
        let dabs: Vec<Dab> = plans
            .into_iter()
            .map(|p| Dab { position: p.position, radius_px: p.radius_px, color: p.color })
            .collect();
        match tool {
            Tool::Paint => canvas.stamp_many(device, queue, &dabs),
            Tool::Erase => canvas.erase_many(device, queue, &dabs),
        }
    }

    pub fn touch_down(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        if let Screen::Painting(state) = &mut self.screen {
            let plans = state.controller.pointer_down(x, y, self.ui.pointer_over_ui());
            Self::apply_dabs(&self.device, &self.queue, &state.canvas, state.tool, plans);
        }
    }

    pub fn second_touch_down(&mut self) {
        if let Screen::Painting(state) = &mut self.screen {
            state.controller.second_pointer_down();
        }
    }

    pub fn touch_move(&mut self, pointers: &[(f32, f32)], pixels_per_point: f32) {
        if let Some(&(x, y)) = pointers.first() {
            self.pending_ui_events.push(egui::Event::PointerMoved(egui::pos2(
                x / pixels_per_point,
                y / pixels_per_point,
            )));
        }
        if let Screen::Painting(state) = &mut self.screen {
            let plans = state.controller.pointer_moved(pointers, self.ui.pointer_over_ui());
            Self::apply_dabs(&self.device, &self.queue, &state.canvas, state.tool, plans);
        }
    }

    pub fn touch_up(&mut self, x: f32, y: f32, pixels_per_point: f32) {
        self.pending_ui_events.push(egui::Event::PointerButton {
            pos: egui::pos2(x / pixels_per_point, y / pixels_per_point),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        self.pending_ui_events.push(egui::Event::PointerGone);
        if let Screen::Painting(state) = &mut self.screen {
            let plans = state.controller.pointer_up(x, y);
            Self::apply_dabs(&self.device, &self.queue, &state.canvas, state.tool, plans);
        }
    }

    fn export_png(device: &wgpu::Device, queue: &wgpu::Queue, canvas: &PaintCanvas, background_mode: BackgroundMode) -> Result<Vec<u8>, String> {
        let (width, height, rgba) = canvas.read_pixels(device, queue);
        let pixels = match background_mode {
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
        let events = std::mem::take(&mut self.pending_ui_events);
        let screen_size_px = (self.config.width, self.config.height);

        // Set only by the Home/Setup arms below, and only acted on AFTER
        // the match ends — reassigning self.screen from inside an arm
        // that's itself matching on &mut self.screen doesn't borrow-
        // check, so the transition has to happen as a separate step once
        // the match's borrow of self.screen has ended.
        enum Transition {
            None,
            ToSetup,
            ToPainting(u32, u32, bool),
        }
        let mut transition = Transition::None;

        let output = match &mut self.screen {
            Screen::Home => {
                let (output, new_canvas_tapped) =
                    self.ui.run_home_frame(events, screen_size_px, pixels_per_point);
                if new_canvas_tapped {
                    transition = Transition::ToSetup;
                }
                output
            }
            Screen::Setup => {
                let (output, chosen) = self.ui.run_setup_frame(
                    self.max_texture_dimension,
                    events,
                    screen_size_px,
                    pixels_per_point,
                );
                if let Some((width, height, pixel_art)) = chosen {
                    transition = Transition::ToPainting(width, height, pixel_art);
                }
                output
            }
            Screen::Painting(state) => {
                // Commits any touch-start that's cleared the pinch-
                // disambiguation window since the last frame — see
                // CanvasController::tick's doc comment. Done before
                // anything else so a freshly-committed dab still shows
                // up in THIS frame's draw, not the next one.
                let tick_dabs = state.controller.tick();
                Self::apply_dabs(&self.device, &self.queue, &state.canvas, state.tool, tick_dabs);

                let aspect = self.config.width as f32 / self.config.height as f32;
                state.canvas.set_camera(&self.queue, state.controller.camera().view_proj(aspect));

                // Read before the mutable camera borrow below —
                // brush_preset() and camera_mut() both borrow
                // state.controller, and only one mutable borrow of it
                // can be alive at a time.
                let current_radius = state.controller.brush_preset().radius_px;
                let output = self.ui.run_frame(
                    state.controller.camera_mut(),
                    state.background_mode,
                    state.tool,
                    current_radius,
                    events,
                    screen_size_px,
                    pixels_per_point,
                );

                if self.ui.take_background_toggle_requested() {
                    state.background_mode = match state.background_mode {
                        BackgroundMode::Transparent => BackgroundMode::white(),
                        BackgroundMode::Solid(_) => BackgroundMode::Transparent,
                    };
                    state.canvas.set_background_mode(&self.queue, state.background_mode);
                }

                if self.ui.take_tool_toggle_requested() {
                    state.tool = state.tool.toggled();
                }

                if let Some(radius) = self.ui.take_brush_radius_change() {
                    state.controller.brush_preset_mut().radius_px = radius;
                }

                if self.ui.take_export_request() {
                    match Self::export_png(&self.device, &self.queue, &state.canvas, state.background_mode) {
                        Ok(bytes) => self.pending_export = Some(bytes),
                        Err(e) => {
                            log::error!("PNG export failed: {e}");
                            self.ui.set_export_status(format!("Export failed: {e}"));
                        }
                    }
                }

                output
            }
        };

        match transition {
            Transition::None => {}
            Transition::ToSetup => {
                self.screen = Screen::Setup;
            }
            Transition::ToPainting(width, height, pixel_art) => {
                let width = width.clamp(64, self.max_texture_dimension);
                let height = height.clamp(64, self.max_texture_dimension);
                let canvas =
                    PaintCanvas::new(&self.device, &self.queue, self.config.format, DEPTH_FORMAT, width, height, pixel_art);
                let controller = CanvasController::new(
                    canvas.half_extents(),
                    canvas.texture_size_px(),
                    (self.config.width as f32, self.config.height as f32),
                );
                self.screen = Screen::Painting(PaintingState {
                    canvas,
                    controller,
                    background_mode: BackgroundMode::Transparent,
                    tool: Tool::Paint,
                });
            }
        }

        self.present(output, pixels_per_point, clear_color);
    }

    /// Shared by both screens — a `Screen::Setup` frame has no canvas to
    /// draw, but still needs the same surface-acquire/egui-render/submit
    /// machinery as a `Screen::Painting` one.
    fn present(&mut self, output: egui::FullOutput, pixels_per_point: f32, clear_color: wgpu::Color) {
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

            if let Screen::Painting(state) = &self.screen {
                state.canvas.draw(&mut pass);
            }

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
