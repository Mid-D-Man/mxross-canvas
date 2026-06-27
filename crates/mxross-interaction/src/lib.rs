// crates/mxross-interaction/src/lib.rs
//! MxRoss Canvas interaction layer.
//!
//! Owns the camera + brush engine together and decides what a pointer
//! event *means* — orbit the camera, pinch-zoom, or paint a stroke —
//! given only plain `f32` coordinates and a `bool` for "did the UI claim
//! this pointer." That last part matters: this crate has no concept of
//! egui (or any UI framework) at all, so it can't check pointer-claim
//! state itself — the caller computes that however its own UI layer
//! works and passes the answer in.
//!
//! This is the actual payoff of "things that aren't platform-specific
//! get their own crate": a future iOS or desktop platform crate
//! translates its own native pointer events into calls to
//! `pointer_down`/`pointer_moved`/`pointer_up`/`second_pointer_down` and
//! gets identical orbit/zoom/paint behavior for free — none of this
//! logic needs to exist twice, even though it happens to have been
//! written and tested against Android first.

use mxross_brush::{BrushEngine, BrushPreset, DabPlan};
use mxross_camera::{CameraMode, OrbitCamera};

pub struct CanvasController {
    camera: OrbitCamera,
    brush_engine: BrushEngine,
    canvas_half_size: f32,
    screen_size: (f32, f32),
    /// Last single-finger pointer position, in window pixel coordinates
    /// — used for camera-drag deltas only; the brush engine tracks its
    /// own stroke history internally.
    last_pointer: Option<(f32, f32)>,
    /// Distance between the first two active pointers, as of the last
    /// `pointer_moved` call. None whenever fewer than two are down.
    last_pinch_distance: Option<f32>,
    /// True for the duration of a single-pointer stroke that started
    /// somewhere paintable. Decided once, at pointer-down, from both the
    /// camera state and whether that specific touch landed on the canvas
    /// plane at all.
    is_painting: bool,
}

impl CanvasController {
    /// `canvas_half_size` and `canvas_texture_size_px` describe the
    /// paint canvas this controller maps pointer input onto — callers
    /// get these from whatever owns the actual canvas (e.g.
    /// `mxross-render-gpu`'s `PaintCanvas::half_size`/`texture_size_px`),
    /// without this crate needing to depend on that crate directly.
    pub fn new(canvas_half_size: f32, canvas_texture_size_px: f32, screen_size: (f32, f32)) -> Self {
        Self {
            camera: OrbitCamera::new(),
            brush_engine: BrushEngine::new(BrushPreset::default_ink(), canvas_texture_size_px),
            canvas_half_size,
            screen_size,
            last_pointer: None,
            last_pinch_distance: None,
            is_painting: false,
        }
    }

    /// Call whenever the window/surface size changes.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.screen_size = (width, height);
    }

    pub fn camera(&self) -> &OrbitCamera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut OrbitCamera {
        &mut self.camera
    }

    pub fn brush_preset_mut(&mut self) -> &mut BrushPreset {
        self.brush_engine.preset_mut()
    }

    fn can_paint(&self) -> bool {
        self.camera.mode() == CameraMode::LockedOrtho && self.camera.is_front_view()
    }

    /// Maps a pointer position (raw window pixels) to canvas UV (0..1,
    /// top-left origin) — only valid in the camera's locked-ortho front
    /// view (a simple orthographic unproject, not a general ray-plane
    /// intersection; that generalization is for whenever painting from
    /// other angles or in FreeOrbit gets built).
    fn canvas_uv_at(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let aspect = self.screen_size.0 / self.screen_size.1;
        let (half_width, half_height) = self.camera.ortho_half_extents(aspect);

        let ndc_x = (x / self.screen_size.0) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y / self.screen_size.1) * 2.0;

        let world_x = ndc_x * half_width;
        let world_y = ndc_y * half_height;

        let u = (world_x / self.canvas_half_size + 1.0) / 2.0;
        let v = (1.0 - world_y / self.canvas_half_size) / 2.0;

        if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
            Some((u, v))
        } else {
            None
        }
    }

    /// First finger down. `ui_claims_pointer` is the caller's own UI
    /// layer's answer to "is this pointer already being used by a
    /// widget" — one-frame-stale is fine here (and is exactly what the
    /// Android caller currently passes), same accepted caveat as before:
    /// tapping directly on a UI element could in principle start a
    /// stroke before the UI catches up the following frame.
    pub fn pointer_down(&mut self, x: f32, y: f32, ui_claims_pointer: bool) -> Vec<DabPlan> {
        self.last_pointer = Some((x, y));
        self.is_painting = self.can_paint() && !ui_claims_pointer;

        if self.is_painting {
            match self.canvas_uv_at(x, y) {
                Some(uv) => return self.brush_engine.start_stroke(uv),
                // Landed in the margin outside the canvas plane — don't
                // start a stroke for it at all, so a drag that later
                // wanders onto the canvas still doesn't paint (you have
                // to *start* on the canvas).
                None => self.is_painting = false,
            }
        }
        Vec::new()
    }

    /// A second pointer touched down — treated as the start of a pinch,
    /// not painting. Cancels any in-progress stroke so the pinch doesn't
    /// leave a stray dab where the first pointer happened to be.
    pub fn second_pointer_down(&mut self) {
        if self.is_painting {
            self.brush_engine.cancel_stroke();
        }
        self.is_painting = false;
    }

    /// `pointers` are ALL currently active pointers. Two or more drive
    /// pinch-to-zoom instead of painting or camera orbit; only the first
    /// pointer's position is used otherwise.
    pub fn pointer_moved(&mut self, pointers: &[(f32, f32)], ui_claims_pointer: bool) -> Vec<DabPlan> {
        let Some(&(x, y)) = pointers.first() else { return Vec::new() };

        if pointers.len() >= 2 {
            let (x1, y1) = pointers[1];
            let distance = ((x1 - x).powi(2) + (y1 - y).powi(2)).sqrt();
            if let Some(last) = self.last_pinch_distance {
                self.camera.zoom(distance / last.max(1.0));
            }
            self.last_pinch_distance = Some(distance);
            self.is_painting = false;
            self.last_pointer = Some((x, y));
            return Vec::new();
        }

        self.last_pinch_distance = None;

        let plans = if self.is_painting {
            // If this particular sample lands off-canvas mid-stroke, just
            // skip stamping it — the stroke itself stays alive (unlike
            // pointer_down, which refuses to ever *start* one
            // off-canvas), so painting resumes if the finger wanders back
            // onto the canvas before lifting.
            self.canvas_uv_at(x, y)
                .map(|uv| self.brush_engine.push_point(uv))
                .unwrap_or_default()
        } else {
            if let Some((lx, ly)) = self.last_pointer {
                if !ui_claims_pointer {
                    self.camera.handle_drag(x - lx, y - ly);
                }
            }
            Vec::new()
        };

        self.last_pointer = Some((x, y));
        plans
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) -> Vec<DabPlan> {
        let mut plans = Vec::new();
        if self.is_painting {
            // The lift position is one more real sample, not just a
            // release notification — feed it before flushing the tail.
            if let Some(uv) = self.canvas_uv_at(x, y) {
                plans.extend(self.brush_engine.push_point(uv));
            }
            plans.extend(self.brush_engine.end_stroke());
        }
        self.last_pointer = None;
        self.last_pinch_distance = None;
        self.is_painting = false;
        plans
    }
  }
