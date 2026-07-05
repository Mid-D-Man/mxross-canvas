// crates/mxross-android/src/ui.rs
//! egui integration — the foundation for every future UI element in
//! MxRoss Canvas. There's no winit anywhere in this app, so there's no
//! egui-winit glue to lean on — touch events are translated into
//! `egui::Event`s by hand in gpu.rs and fed in here directly.

use std::time::Instant;

use mxross_camera::{CameraMode, OrbitCamera};
use mxross_render_gpu::BackgroundMode;

use crate::gizmo;
use crate::gpu::Tool;

pub struct AppUi {
    ctx: egui::Context,
    pointer_over_ui: bool,
    background_toggle_requested: bool,
    tool_toggle_requested: bool,
    /// Set whenever the brush-size slider is dragged to a new value this
    /// frame — `Option` rather than a bare `f32` so `take_...` can tell
    /// "no change happened" apart from "changed back to the same value
    /// it already was," even though the latter would be harmless either
    /// way. Consistent with the take-and-clear pattern every other
    /// one-shot UI request in this struct already uses.
    brush_radius_change: Option<f32>,
    export_requested: bool,
    /// The actual result of the last export attempt — set from outside
    /// (gpu.rs, after lib.rs has tried writing the file) since this is
    /// the only place with no other way to surface that to you without
    /// logcat. Persists until the next export attempt overwrites it;
    /// no auto-clear timer, simplest thing that gives a real answer.
    last_export_status: Option<String>,
    start: Instant,
    /// Currently-typed custom width/height on the "New Canvas" setup
    /// screen — persists across frames the way a text field naturally
    /// would, unlike the take-and-clear request flags above which are
    /// one-shot events rather than ongoing state.
    setup_width: u32,
    setup_height: u32,
    setup_pixel_art: bool,
}

/// (label, width, height) shown as one-tap buttons on the setup screen.
/// A mix of square and common non-square sizes, since "custom" already
/// covers everything else — these are just the sizes worth one tap
/// instead of two DragValue drags.
const CANVAS_PRESETS: &[(&str, u32, u32)] = &[
    ("Square — 1024", 1024, 1024),
    ("Square — 2048", 2048, 2048),
    ("Square — 4096", 4096, 4096),
    ("Landscape — 1920 x 1080", 1920, 1080),
    ("Portrait — 1080 x 1920", 1080, 1920),
];

impl AppUi {
    pub fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            pointer_over_ui: false,
            background_toggle_requested: false,
            tool_toggle_requested: false,
            brush_radius_change: None,
            export_requested: false,
            last_export_status: None,
            start: Instant::now(),
            setup_width: 1024,
            setup_height: 1024,
            setup_pixel_art: false,
        }
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    pub fn pointer_over_ui(&self) -> bool {
        self.pointer_over_ui
    }

    pub fn take_background_toggle_requested(&mut self) -> bool {
        std::mem::take(&mut self.background_toggle_requested)
    }

    pub fn take_export_request(&mut self) -> bool {
        std::mem::take(&mut self.export_requested)
    }

    pub fn take_tool_toggle_requested(&mut self) -> bool {
        std::mem::take(&mut self.tool_toggle_requested)
    }

    pub fn take_brush_radius_change(&mut self) -> Option<f32> {
        self.brush_radius_change.take()
    }

    /// Called from gpu.rs once the actual file write (done in lib.rs,
    /// which is the only place that knows the Android storage path) has
    /// either succeeded or failed.
    pub fn set_export_status(&mut self, status: String) {
        self.last_export_status = Some(status);
    }

    /// The entry screen shown before any canvas exists — presets plus a
    /// custom width/height, gated to `[64, max_dimension]` so a typed-in
    /// value can't exceed what this GPU can actually allocate (see
    /// `GpuState::max_texture_dimension`). Returns the chosen
    /// `(width, height, pixel_art)` the frame a preset or "Create Custom
    /// Canvas" gets tapped, `None` every other frame. The pixel-art
    /// checkbox applies to whichever is chosen — presets included, not
    /// just custom sizes, since there's no reason a preset-sized canvas
    /// shouldn't also get hard edges.
    pub fn run_setup_frame(
        &mut self,
        max_dimension: u32,
        events: Vec<egui::Event>,
        screen_size_px: (u32, u32),
        pixels_per_point: f32,
    ) -> (egui::FullOutput, Option<(u32, u32, bool)>) {
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

        let min_dimension = 64_u32.min(max_dimension);
        let mut chosen = None;
        // Rough-centered rather than perfectly centered (no measured
        // panel size available up front) — good enough for a one-off
        // entry screen; `set_width` below at least keeps its own layout
        // consistent regardless of position.
        let panel_pos = egui::pos2(
            (screen_size_points.x / 2.0 - 150.0).max(16.0),
            (screen_size_points.y / 2.0 - 190.0).max(16.0),
        );

        let output = self.ctx.run_ui(raw_input, |ui| {
            egui::Area::new(egui::Id::new("new_canvas_setup"))
                .fixed_pos(panel_pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::default()
                        .fill(egui::Color32::from_black_alpha(230))
                        .corner_radius(8.0)
                        .inner_margin(16.0)
                        .show(ui, |ui| {
                            ui.set_width(300.0);
                            ui.heading("New Canvas");
                            ui.separator();
                            ui.checkbox(&mut self.setup_pixel_art, "Pixel art mode (hard edges, no smoothing)");
                            ui.separator();
                            ui.label("Presets");
                            for (label, width, height) in CANVAS_PRESETS {
                                if ui.button(*label).clicked() {
                                    chosen = Some((*width, *height, self.setup_pixel_art));
                                }
                            }
                            ui.separator();
                            ui.label("Custom size (px)");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut self.setup_width)
                                        .range(min_dimension..=max_dimension)
                                        .suffix(" w"),
                                );
                                ui.label("×");
                                ui.add(
                                    egui::DragValue::new(&mut self.setup_height)
                                        .range(min_dimension..=max_dimension)
                                        .suffix(" h"),
                                );
                            });
                            if ui.button("Create Custom Canvas").clicked() {
                                chosen = Some((self.setup_width, self.setup_height, self.setup_pixel_art));
                            }
                        });
                });
        });

        self.pointer_over_ui = self.ctx.egui_wants_pointer_input();
        (output, chosen)
    }

    pub fn run_frame(
        &mut self,
        camera: &mut OrbitCamera,
        background_mode: BackgroundMode,
        tool: Tool,
        brush_radius: f32,
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

        let mode = camera.mode();
        let readout = camera.readout();
        let basis = camera.basis();
        let background_label = match background_mode {
            BackgroundMode::Transparent => "Background: None",
            BackgroundMode::Solid(_) => "Background: White",
        };
        let export_status = self.last_export_status.clone();
        let mut clicked_toggle = false;
        let mut clicked_axis = None;
        let mut clicked_background = false;
        let mut clicked_export = false;
        let mut clicked_focus = false;
        let mut clicked_tool = false;
        let mut new_radius = None;

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

            egui::Area::new(egui::Id::new("focus_canvas"))
                .fixed_pos(egui::pos2(16.0, 216.0))
                .show(ui.ctx(), |ui| {
                    if ui.button("Focus Canvas").clicked() {
                        clicked_focus = true;
                    }
                });

            egui::Area::new(egui::Id::new("tool_toggle"))
                .fixed_pos(egui::pos2(16.0, 256.0))
                .show(ui.ctx(), |ui| {
                    if ui.button(tool.label()).clicked() {
                        clicked_tool = true;
                    }
                });

            egui::Area::new(egui::Id::new("brush_size_slider"))
                .fixed_pos(egui::pos2(16.0, 296.0))
                .show(ui.ctx(), |ui| {
                    // Background panel for the same reason export_status
                    // gets one — a bare slider is hard to see against a
                    // busy painted canvas.
                    egui::Frame::default()
                        .fill(egui::Color32::from_black_alpha(180))
                        .corner_radius(4.0)
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            let mut radius = brush_radius;
                            // 2..=64 canvas-texture px (same unit
                            // BrushPreset::radius_px already uses). 2 is
                            // small-but-usable at the 1024px canvas
                            // resolution; 64 is wide enough to matter
                            // without the slider being mostly dead
                            // space. Tune by feel on-device.
                            let response = ui.add(
                                egui::Slider::new(&mut radius, 2.0..=64.0).text("Brush Size"),
                            );
                            if response.changed() {
                                new_radius = Some(radius);
                            }
                        });
                });

            if let Some(status) = &export_status {
                egui::Area::new(egui::Id::new("export_status"))
                    .fixed_pos(egui::pos2(16.0, 176.0))
                    .show(ui.ctx(), |ui| {
                        // Background panel, not just bare text — a plain
                        // label here would be invisible against a busy
                        // painted canvas behind it.
                        egui::Frame::default()
                            .fill(egui::Color32::from_black_alpha(180))
                            .corner_radius(4.0)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(status).color(egui::Color32::WHITE));
                            });
                    });
            }

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
        if clicked_focus {
            camera.focus_canvas();
        }
        if clicked_tool {
            self.tool_toggle_requested = true;
        }
        if let Some(radius) = new_radius {
            self.brush_radius_change = Some(radius);
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
