// crates/mxross-interaction/src/lib.rs
//! MxRoss Canvas interaction layer.
//!
//! Owns the camera + brush engine together and decides what a pointer
//! event *means* — orbit/pan/zoom the camera, or paint a stroke — given
//! only plain `f32` coordinates and a `bool` for "did the UI claim this
//! pointer."
//!
//! ## Touch-start disambiguation
//!
//! A naive "first finger down = start painting immediately" has a real
//! bug: in a genuine two-finger pinch gesture, the fingers never land at
//! *exactly* the same instant — finger A's Down event always arrives
//! some milliseconds before finger B's PointerDown. If finger A
//! immediately commits a dab, that ink is already on the canvas before
//! there's any way to know a second finger was coming at all; canceling
//! the *stroke* once finger B arrives doesn't undo the dab that already
//! landed. The fix: hold the first touch as `pending_start` rather than
//! painting immediately, and only commit it once either (a) a short
//! window passes with no second finger (`PAINT_START_DELAY`,
//! confirmed via `tick`, called every frame), or (b) the finger lifts
//! first (`pointer_up`) — lifting is itself proof no pinch is coming for
//! this gesture, so that case commits immediately rather than waiting
//! out the window pointlessly. If a second finger arrives at any point
//! before either of those, `second_pointer_down` drops the pending start
//! outright — no dab is ever created.

use std::time::{Duration, Instant};

use mxross_brush::{BrushEngine, BrushPreset, DabPlan};
use mxross_camera::{CameraMode, OrbitCamera};

/// How long a single-finger touch waits before committing its first dab,
/// in case a second finger is about to land and turn this into a pinch
/// instead. A feel constant, not something derivable from documentation
/// — typical real-world pinch gestures land both fingers within a few
/// tens of milliseconds of each other, so this gives comfortable margin
/// without adding noticeable lag to an intentional single stroke. Worth
/// tuning by feel on-device rather than treating as fixed.
const PAINT_START_DELAY: Duration = Duration::from_millis(70);

struct PendingStart {
    position: (f32, f32),
    started_at: Instant,
}

pub struct CanvasController {
    camera: OrbitCamera,
    brush_engine: BrushEngine,
    canvas_half_size: f32,
    screen_size: (f32, f32),
    last_pointer: Option<(f32, f32)>,
    last_pinch_distance: Option<f32>,
    last_pinch_midpoint: Option<(f32, f32)>,
    is_painting: bool,
    /// A first-finger touch that's eligible to paint but hasn't been
    /// committed yet — see module doc comment.
    pending_start: Option<PendingStart>,
}

impl CanvasController {
    pub fn new(canvas_half_size: f32, canvas_texture_size_px: f32, screen_size: (f32, f32)) -> Self {
        Self {
            camera: OrbitCamera::new(),
            brush_engine: BrushEngine::new(BrushPreset::default_ink(), canvas_texture_size_px),
            canvas_half_size,
            screen_size,
            last_pointer: None,
            last_pinch_distance: None,
            last_pinch_midpoint: None,
            is_painting: false,
            pending_start: None,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.screen_size = (width, height);
    }

    pub fn camera(&self) -> &OrbitCamera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut OrbitCamera {
        &mut self.camera
    }

    /// Immutable counterpart to `brush_preset_mut()` — needed so
    /// `gpu.rs` can read the current radius for the UI slider in the
    /// same frame it also needs `camera_mut()`; without a separate
    /// immutable accessor, both would have to borrow `self.controller`
    /// mutably at once.
    pub fn brush_preset(&self) -> &BrushPreset {
        self.brush_engine.preset()
    }

    pub fn brush_preset_mut(&mut self) -> &mut BrushPreset {
        self.brush_engine.preset_mut()
    }

    fn can_paint(&self) -> bool {
        self.camera.mode() == CameraMode::LockedOrtho && self.camera.is_front_view()
    }

    /// Shared screen-to-canvas-UV math. Returns UV *unclamped* — callers
    /// decide whether an out-of-[0,1] result means "not on the canvas"
    /// (use `canvas_uv_at`) or "still draw, just pinned to the edge"
    /// (use `canvas_uv_clamped`).
    fn canvas_uv_raw(&self, x: f32, y: f32) -> (f32, f32) {
        let aspect = self.screen_size.0 / self.screen_size.1;
        let (half_width, half_height) = self.camera.ortho_half_extents(aspect);
        // pan_offset shifts the look-at target away from the origin —
        // without adding it here, the UV mapping treats the camera as
        // always looking at (0,0) in world space, so panning the canvas
        // left then drawing puts ink somewhere off to the right.
        let (pan_x, pan_y) = self.camera.pan_offset_xy();

        let ndc_x = (x / self.screen_size.0) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y / self.screen_size.1) * 2.0;

        let world_x = ndc_x * half_width + pan_x;
        let world_y = ndc_y * half_height + pan_y;

        let u = (world_x / self.canvas_half_size + 1.0) / 2.0;
        let v = (1.0 - world_y / self.canvas_half_size) / 2.0;
        (u, v)
    }

    /// `None` unless `(x, y)` actually lands on the canvas. Used for
    /// "is this position on the canvas at all" decisions: whether a tap
    /// is eligible to start a stroke, whether a pending touch has
    /// crossed onto the canvas yet.
    fn canvas_uv_at(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (u, v) = self.canvas_uv_raw(x, y);
        if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
            Some((u, v))
        } else {
            None
        }
    }

    /// Always returns a UV, pinned to the canvas edge if `(x, y)` is
    /// outside it. Used to *continue* an already-committed stroke: once
    /// painting has started, a finger that drifts off the canvas edge
    /// should keep drawing right up to (and along) the border instead of
    /// silently stopping, which is what leaves a gap at the edge where
    /// the last in-bounds dab landed short of where the finger actually
    /// left the canvas.
    fn canvas_uv_clamped(&self, x: f32, y: f32) -> (f32, f32) {
        let (u, v) = self.canvas_uv_raw(x, y);
        (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
    }

    /// Force-commits whatever's pending, regardless of how much time has
    /// elapsed. Called unconditionally from `pointer_up` (lifting proves
    /// no pinch is coming) and conditionally from `maybe_commit_pending`
    /// once the delay window has actually passed.
    /// Deliberately does NOT drop `pending_start` when the position isn't
    /// on the canvas yet — a touch that starts outside the canvas and
    /// drags inward should start painting the moment it crosses the
    /// edge, not be discarded the first time this is called while it's
    /// still off-canvas. `pointer_moved` keeps `pending.position` fresh
    /// every frame (see below), so this re-checks the *current* position
    /// each time, not the original touch-down position.
    fn commit_pending(&mut self) -> Vec<DabPlan> {
        let Some(pending) = self.pending_start.as_ref() else { return Vec::new() };
        let Some(uv) = self.canvas_uv_at(pending.position.0, pending.position.1) else {
            return Vec::new();
        };
        self.pending_start = None;
        self.is_painting = true;
        self.brush_engine.start_stroke(uv)
    }

    fn maybe_commit_pending(&mut self) -> Vec<DabPlan> {
        let ready = self
            .pending_start
            .as_ref()
            .is_some_and(|p| p.started_at.elapsed() >= PAINT_START_DELAY);
        if ready {
            self.commit_pending()
        } else {
            Vec::new()
        }
    }

    /// Call once per frame regardless of input events — this is what
    /// lets a perfectly still single-finger touch (no Move samples at
    /// all) still get its dab committed once the disambiguation window
    /// passes, rather than only ever being checked from inside
    /// `pointer_moved`.
    pub fn tick(&mut self) -> Vec<DabPlan> {
        self.maybe_commit_pending()
    }

    /// First finger down. Does NOT paint immediately — see module doc
    /// comment. `ui_claims_pointer` is the caller's own UI layer's
    /// answer to "is this pointer already being used by a widget."
    pub fn pointer_down(&mut self, x: f32, y: f32, ui_claims_pointer: bool) -> Vec<DabPlan> {
        self.last_pointer = Some((x, y));
        self.is_painting = false;
        self.pending_start = None;

        // No canvas_uv_at(x, y).is_some() gate here on purpose — a touch
        // that starts outside the canvas (e.g. dragging in from off the
        // edge) still needs to become a pending start so it can be
        // committed once it drags onto the canvas. In LockedOrtho,
        // camera drag is a no-op anyway, so holding a pending start that
        // never resolves costs nothing.
        if self.can_paint() && !ui_claims_pointer {
            self.pending_start = Some(PendingStart { position: (x, y), started_at: Instant::now() });
        }
        Vec::new()
    }

    /// A second pointer touched down — drops any pending (not yet
    /// committed) start outright, and cancels an already-committed
    /// stroke too, covering both timings.
    pub fn second_pointer_down(&mut self) {
        if self.is_painting {
            self.brush_engine.cancel_stroke();
        }
        self.is_painting = false;
        self.pending_start = None;
    }

    /// `pointers` are ALL currently active pointers. Two or more drive
    /// pinch-to-zoom AND two-finger pan (tracked via the pair's midpoint)
    /// instead of painting or camera orbit; only the first pointer's
    /// position is used otherwise.
    pub fn pointer_moved(&mut self, pointers: &[(f32, f32)], ui_claims_pointer: bool) -> Vec<DabPlan> {
        let Some(&(x, y)) = pointers.first() else { return Vec::new() };

        if pointers.len() >= 2 {
            let (x1, y1) = pointers[1];
            let distance = ((x1 - x).powi(2) + (y1 - y).powi(2)).sqrt();
            let midpoint = ((x + x1) * 0.5, (y + y1) * 0.5);

            if let Some(last_dist) = self.last_pinch_distance {
                self.camera.zoom(distance / last_dist.max(1.0));
            }
            if let Some(last_mid) = self.last_pinch_midpoint {
                self.camera.pan(
                    midpoint.0 - last_mid.0,
                    midpoint.1 - last_mid.1,
                    self.screen_size.0,
                    self.screen_size.1,
                );
            }
            self.last_pinch_distance = Some(distance);
            self.last_pinch_midpoint = Some(midpoint);

            if self.is_painting {
                self.brush_engine.cancel_stroke();
            }
            self.is_painting = false;
            self.pending_start = None;
            self.last_pointer = Some((x, y));
            return Vec::new();
        }

        self.last_pinch_distance = None;
        self.last_pinch_midpoint = None;

        // Keep the pending start's position fresh so a touch that began
        // off-canvas commits at wherever it actually is now, not the
        // stale off-canvas down position — this is what lets it commit
        // the instant it crosses onto the canvas.
        if let Some(pending) = self.pending_start.as_mut() {
            pending.position = (x, y);
        }

        let mut plans = self.maybe_commit_pending();

        if self.is_painting {
            // Clamped, not the strict Option version: once a stroke is
            // committed, a finger drifting off the canvas edge should
            // keep drawing pinned to the border rather than leaving a
            // gap between the last in-bounds dab and the edge.
            let uv = self.canvas_uv_clamped(x, y);
            plans.extend(self.brush_engine.push_point(uv));
        } else if self.pending_start.is_none() {
            // Still genuinely ambiguous (pending_start.is_some()) ->
            // don't orbit either, just wait. Once neither painting nor
            // pending, fall through to ordinary camera drag (a no-op in
            // LockedOrtho regardless, via handle_drag's own mode check).
            if let Some((lx, ly)) = self.last_pointer {
                if !ui_claims_pointer {
                    self.camera.handle_drag(x - lx, y - ly);
                }
            }
        }

        self.last_pointer = Some((x, y));
        plans
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) -> Vec<DabPlan> {
        // Lifting is proof no second finger is coming for this gesture —
        // force-commit a still-pending tap immediately rather than
        // waiting out (or silently dropping) the disambiguation window.
        let mut plans = self.commit_pending();

        if self.is_painting {
            let uv = self.canvas_uv_clamped(x, y);
            plans.extend(self.brush_engine.push_point(uv));
            plans.extend(self.brush_engine.end_stroke());
        }

        self.last_pointer = None;
        self.last_pinch_distance = None;
        self.last_pinch_midpoint = None;
        self.is_painting = false;
        self.pending_start = None;
        plans
    }
                     }
