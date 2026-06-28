// crates/mxross-android/src/ui.rs
//! egui integration — the foundation for every future UI element in
//! MxRoss Canvas. There's no winit anywhere in this app, so there's no
//! egui-winit glue to lean on — touch events are translated into
//! `egui::Event`s by hand in gpu.rs and fed in here directly.

use std::time::Instant;

use mxross_camera::{CameraMode, OrbitCamera};
use mxross_render_gpu::BackgroundMode;

use crate::gizmo;

pub struct AppUi {
    ctx: egui::Context,
    pointer_over_ui: bool,
    background_toggle_requested: bool,
    export_requested: bool,
    start: Instant,
}

impl AppUi {
    pub fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            pointer_over_ui: false,
            background_toggle_requested: false,
            export_requested: false,
            start: Instant::now(),
        }
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// True if egui claimed the pointer this frame (hovering/dragging a
    /// widget). One-frame-stale by construction (reflects the previous
    /// call to `run_frame`).
    pub fn pointer_over_ui(&self) -> bool {
        self.pointer_over_ui
    }

    pub fn take_background_toggle_requested(&mut self) -> bool {
        std::mem::take(&mut self.background_toggle_requested)
    }

    pub fn take_export_request(&mut self) -> bool {
        std::mem::take(&mut self.export_requested)
    }

    pub fn run_frame(
        &mut self,
        camera: &mut OrbitCamera,
        background_mode: BackgroundMode,
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

        // Read everything before the closure, apply any resulting
        // changes after it returns — see earlier comments on why
        // (capturing `camera` itself inside the closure while also
        // wanting to read it for labels would fight the borrow checker
        // for no real benefit).
        let mode = camera.mode();
        let readout = camera.readout();
        let basis = camera.basis();
        let background_label = match background_mode {
            BackgroundMode::Transparent => "Background: None",
            BackgroundMode::Solid(_) => "Background: White",
        };
        let mut clicked_toggle = false;
        let mut clicked_axis = None;
        let mut clicked_background = false;
        let mut clicked_export = false;

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

            egui::Area::new(egui::Id::new("background_toggle"))
                .fixed_pos(egui::pos2(16.0, 96.0))
                .show(ui.ctx(), |ui| {
                    if ui.button(background_label).clicked() {
                        clicked_background = true;
                    }
                });

            egui::Area::new(egui::Id::new("export_button"))
                .fixed_pos(egui::pos2(16.0, 136.0))
                .show(ui.ctx(), |ui| {
                    if ui.button("Export PNG").clicked() {
                        clicked_export = true;
                    }
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
        if clicked_background {
            self.background_toggle_requested = true;
        }
        if clicked_export {
            self.export_requested = true;
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
