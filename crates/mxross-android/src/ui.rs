// crates/mxross-android/src/ui.rs
//! egui integration — the foundation for every future UI element in
//! MxRoss Canvas (tool palette, layers panel, brush settings, etc.), not
//! just this one camera-mode button. There's no winit anywhere in this
//! app, so there's no egui-winit glue to lean on — touch events are
//! translated into `egui::Event`s by hand in gpu.rs and fed in here
//! directly.
//!
//! Currently this builds exactly one widget (the locked-ortho/free-orbit
//! toggle). As more UI lands, this is the natural place to split "egui
//! plumbing" from "the actual widgets" — doing that split now, with only
//! one widget to learn from, would be guessing at the wrong abstraction.

use std::time::Instant;

use crate::camera::{CameraMode, OrbitCamera};

pub struct AppUi {
    ctx: egui::Context,
    pointer_over_ui: bool,
    start: Instant,
}

impl AppUi {
    pub fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            pointer_over_ui: false,
            start: Instant::now(),
        }
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// True if egui claimed the pointer this frame (hovering/dragging a
    /// widget) — checked by GpuState before applying camera-orbit drag
    /// deltas, so dragging on the toggle button doesn't also spin the
    /// camera underneath it. One-frame-stale by construction (reflects
    /// the previous call to `run_frame`), which only matters if a touch
    /// goes down AND drags within the same ~16ms tick — not worth
    /// chasing for a single button.
    pub fn pointer_over_ui(&self) -> bool {
        self.pointer_over_ui
    }

    pub fn run_frame(
        &mut self,
        camera: &mut OrbitCamera,
        events: Vec<egui::Event>,
        screen_size_px: (u32, u32),
        pixels_per_point: f32,
    ) -> egui::FullOutput {
        self.ctx.set_pixels_per_point(pixels_per_point);

        let screen_size_points = egui::vec2(
            screen_size_px.0 as f32 / pixels_per_point,
            screen_size_px.1 as f32 / pixels_per_point,
        );

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_size_points)),
            time: Some(self.start.elapsed().as_secs_f64()),
            events,
            ..Default::default()
        };

        // Read the mode and write back any click *outside* the closure —
        // capturing `camera` itself inside the closure (a &mut borrow)
        // while also wanting to read it for the label would fight the
        // borrow checker for no real benefit.
        let mode = camera.mode();
        let mut clicked = false;

        let output = self.ctx.run_ui(raw_input, |ui| {
            egui::Area::new(egui::Id::new("camera_mode_toggle"))
                .fixed_pos(egui::pos2(16.0, 16.0))
                .show(ui.ctx(), |ui| {
                    let label = match mode {
                        CameraMode::LockedOrtho => "Locked Ortho",
                        CameraMode::FreeOrbit => "Free Orbit",
                    };
                    if ui.button(label).clicked() {
                        clicked = true;
                    }
                });
        });

        if clicked {
            camera.toggle_mode();
        }

        self.pointer_over_ui = self.ctx.egui_wants_pointer_input();

        output
    }
}

impl Default for AppUi {
    fn default() -> Self {
        Self::new()
    }
  }
