// crates/mxross-android/src/ui.rs
//! egui integration — the foundation for every future UI element in
//! MxRoss Canvas (tool palette, layers panel, brush settings, etc.), not
//! just these camera widgets. There's no winit anywhere in this app, so
//! there's no egui-winit glue to lean on — touch events are translated
//! into `egui::Event`s by hand in gpu.rs and fed in here directly.
//!
//! Builds two widgets right now (mode toggle + camera readout) plus the
//! axis gizmo (gizmo.rs). As more UI lands, this is the natural place to
//! split "egui plumbing" from "the actual widgets" — doing that split
//! now would still be guessing at the wrong abstraction with only a
//! couple of widgets to learn from.

use std::time::Instant;

use mxross_camera::{CameraMode, OrbitCamera};

use crate::gizmo;

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
    /// widget) — checked by GpuState (via mxross-interaction's
    /// `CanvasController`) before applying camera-orbit drag deltas, so
    /// dragging on a widget doesn't also spin the camera underneath it.
    /// One-frame-stale by construction (reflects the previous call to
    /// `run_frame`).
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

        // Read everything we need from `camera` *before* the closure —
        // capturing `camera` itself inside the closure (a &mut borrow)
        // while also wanting to read it for the labels/gizmo would fight
        // the borrow checker for no real benefit. Any resulting changes
        // (toggle click, gizmo click) are collected into locals and
        // applied to `camera` *after* the closure returns instead.
        let mode = camera.mode();
        let readout = camera.readout();
        let basis = camera.basis();
        let mut clicked_toggle = false;
        let mut clicked_axis = None;

        let output = self.ctx.run_ui(raw_input, |ui| {
            egui::Area::new(egui::Id::new("camera_mode_toggle"))
                .fixed_pos(egui::pos2(16.0, 16.0))
                .show(ui.ctx(), |ui| {
                    let label = match mode {
                        CameraMode::LockedOrtho => "Locked Ortho",
                        CameraMode::FreeOrbit => "Free Orbit",
                    };
                    if ui.button(label).clicked() {
                        clicked_toggle = true;
                    }
                });

            egui::Area::new(egui::Id::new("camera_readout"))
                .fixed_pos(egui::pos2(16.0, 56.0))
                .show(ui.ctx(), |ui| {
                    ui.label(readout.as_str());
                });

            egui::Area::new(egui::Id::new("camera_gizmo"))
                .fixed_pos(egui::pos2(screen_size_points.x - 136.0, 16.0))
                .show(ui.ctx(), |ui| {
                    clicked_axis = gizmo::show(ui, basis);
                });
        });

        if clicked_toggle {
            camera.toggle_mode();
        }
        if let Some(axis) = clicked_axis {
            camera.snap_to_axis(axis);
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
