// crates/mxross-android/src/gpu.rs
//! wgpu GPU render path for MxRoss Canvas on Android — Instance -> Surface
//! -> Adapter -> Device -> Queue, then one clear-color render pass per
//! frame. This replaces the original NativeWindow software pixel-buffer
//! loop now that the GPU path is confirmed end-to-end.
//!
//! ## The HasDisplayHandle gap
//!
//! `ndk::native_window::NativeWindow` (0.9, default `rwh_06` feature)
//! implements `raw_window_handle::HasWindowHandle` — confirmed directly
//! from its source — but NOT `HasDisplayHandle`. That's not a missing
//! feature to work around; Android genuinely has no separate "display
//! connection" object the way X11/Wayland do, so `raw-window-handle`
//! models it as `AndroidDisplayHandle {}` — an explicitly empty marker
//! struct (confirmed in raw-window-handle 0.6.2's source) with a
//! `DisplayHandle::android()` constructor.
//!
//! `WindowHandles` below just bundles a `NativeWindow` with that empty
//! marker so the combined type satisfies wgpu's `DisplayAndWindowHandle`
//! bound, which means `Instance::create_surface` can be called directly —
//! no unsafe `SurfaceTargetUnsafe::RawHandle` path needed.

use ndk::native_window::NativeWindow;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

/// Bundles a `NativeWindow` with the (empty) Android display handle.
/// Takes ownership of the window — wgpu's `Surface` keeps this alive
/// internally for as long as the surface itself lives, so nothing else
/// needs to hold onto it.
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
/// lib.rs) — more churn than strictly necessary (the `Instance` in
/// particular could outlive window swaps) but it's the simplest correct
/// thing to ship for this first on-device GPU milestone. Revisit only if
/// window-swap churn turns out to actually cost real frame time.
pub struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl GpuState {
    /// Builds the full Instance -> Surface -> Adapter -> Device chain for
    /// `window` and configures the surface at its current size.
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

        Ok(Self { surface, device, queue, config })
    }

    /// Re-applies the config at a new size. Wired up to Android's
    /// `WindowResized` event (see lib.rs) — the window itself doesn't get
    /// torn down on resize the way it does on `TerminateWindow`, so this
    /// is the lighter-weight path that doesn't touch the Instance/Adapter/
    /// Device at all.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Clears the frame to `color` — the GPU-equivalent of the original
    /// NativeWindow pixel-lock dot test's first proof point. Brush
    /// stamping replaces this clear-only render pass once the camera and
    /// a test primitive land.
    ///
    /// `Outdated`/`Lost` are deliberately just skipped for now rather than
    /// recovered from (re-configure / recreate-surface respectively) —
    /// fine for this milestone since InitWindow already rebuilds GpuState
    /// from scratch on any real window swap; proper recovery is a later
    /// robustness pass, not something blocking "does the clear reach the
    /// screen on the A13".
    pub fn render_clear(&self, color: wgpu::Color) {
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
                label: Some("mxross-android clear encoder"),
            });

        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mxross-android clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }
  }
