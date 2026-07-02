// crates/mxross-camera/src/orbit.rs
//! Touch-driven camera — two modes, matching the decided MxRoss Canvas
//! viewport architecture:
//!   - `LockedOrtho` (default): flat orthographic view, no orbit drag,
//!     pinch-to-zoom, two-finger pan, and axis-gizmo snapping.
//!   - `FreeOrbit`: perspective orbit, single-finger drag + pinch-zoom +
//!     axis-gizmo snapping. No panning in this mode yet — see `pan`'s
//!     doc comment for why that's scoped out rather than half-built.
//! Both modes share one yaw/pitch/radius/pan_offset state — "locked"
//! only means "no free dragging", not "frozen direction or position";
//! snapping to a cardinal axis re-points either mode at it, exactly like
//! Blender's numpad views.

use mxross_math::{Mat4, Vec3};

use crate::Camera;

/// Pixels-to-radians drag sensitivity. If orbiting feels backwards on
/// device, flip the sign on the corresponding line in `handle_drag` —
/// the "natural" drag direction is a matter of feel, not something to
/// derive from first principles.
const ORBIT_SENSITIVITY: f32 = 0.005;

/// Keeps free-drag pitch away from the poles — at exactly ±90° `yaw`
/// stops doing anything mid-drag, which feels broken in a way a discrete
/// gizmo snap to the same angle does not (see `up_vector`'s pole
/// handling, which is what makes landing exactly on ±90° safe for a
/// *snap* specifically).
const MAX_DRAG_PITCH: f32 = 85.0 / 180.0 * std::f32::consts::PI;

/// Pinch-zoom clamp on `radius`. Also bounds the LockedOrtho half-height,
/// since both modes share this one knob. Widened from the original
/// (1.0, 20.0) — these are still just a reasonable starting point for
/// "more zoom range in both directions", not a measured-correct value;
/// easy to retune further if it still feels off on-device.
const MIN_RADIUS: f32 = 0.5;
const MAX_RADIUS: f32 = 30.0;

/// `LockedOrtho`'s half-height as a fraction of `radius` — chosen so the
/// default radius (4.0) gives a half-height of 2.5, slightly larger than
/// the paint canvas's own half-size (2.0) so the canvas doesn't fill the
/// entire screen at default zoom.
const ORTHO_HALF_HEIGHT_FACTOR: f32 = 0.625;

/// A cardinal world axis, as offered by the gizmo (mxross-android's
/// gizmo.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Axis {
    pub fn direction(self) -> Vec3 {
        match self {
            Axis::PosX => Vec3::X,
            Axis::NegX => Vec3::NEG_X,
            Axis::PosY => Vec3::Y,
            Axis::NegY => Vec3::NEG_Y,
            Axis::PosZ => Vec3::Z,
            Axis::NegZ => Vec3::NEG_Z,
        }
    }

    /// (yaw, pitch) in degrees that puts the eye out along this axis,
    /// looking back toward the origin.
    fn yaw_pitch_degrees(self) -> (f32, f32) {
        match self {
            Axis::PosZ => (0.0, 0.0),
            Axis::NegZ => (180.0, 0.0),
            Axis::PosX => (90.0, 0.0),
            Axis::NegX => (-90.0, 0.0),
            Axis::PosY => (0.0, 90.0),
            Axis::NegY => (0.0, -90.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    LockedOrtho,
    FreeOrbit,
}

pub struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    radius: f32,
    mode: CameraMode,
    /// What the camera looks at — replaces a hardcoded `Vec3::ZERO`
    /// target. Shifted by `pan()`. The eye is always `radius` away from
    /// this point, in the `direction()` computed from yaw/pitch.
    pan_offset: Vec3,
}

impl OrbitCamera {
    pub fn new() -> Self {
        Self {
            // Front view by default — matches the original hardcoded
            // LockedOrtho eye exactly.
            yaw: 0.0,
            pitch: 0.0,
            radius: 4.0,
            mode: CameraMode::LockedOrtho,
            pan_offset: Vec3::ZERO,
        }
    }

    pub fn mode(&self) -> CameraMode {
        self.mode
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            CameraMode::LockedOrtho => CameraMode::FreeOrbit,
            CameraMode::FreeOrbit => CameraMode::LockedOrtho,
        };
    }

    /// `dx`/`dy` are the raw pixel delta since the last touch sample.
    /// No-op while locked.
    pub fn handle_drag(&mut self, dx: f32, dy: f32) {
        if self.mode != CameraMode::FreeOrbit {
            return;
        }
        self.yaw += dx * ORBIT_SENSITIVITY;
        self.pitch = (self.pitch - dy * ORBIT_SENSITIVITY).clamp(-MAX_DRAG_PITCH, MAX_DRAG_PITCH);
    }

    /// `factor` is `current_pinch_distance / previous_pinch_distance` —
    /// >1.0 zooms in, <1.0 zooms out. Works in both camera modes since
    /// both `view_proj` branches read `radius`.
    pub fn zoom(&mut self, factor: f32) {
        if factor <= 0.0 || !factor.is_finite() {
            return;
        }
        self.radius = (self.radius / factor).clamp(MIN_RADIUS, MAX_RADIUS);
    }

    /// Two-finger drag pan, in raw screen pixels (`screen_dx`/`screen_dy`
    /// = midpoint delta since the last sample). LockedOrtho only — true
    /// panning in FreeOrbit needs a defined depth plane to be pixel-
    /// accurate (1:1 finger tracking depends on how far away the thing
    /// you're "grabbing" actually is), and there's no real 3D scene
    /// content yet to anchor that against. `focus_canvas` is the
    /// FreeOrbit-safe escape hatch for "I've lost the canvas" instead.
    ///
    /// Sign derivation: shifting `pan_offset` moves both eye and target
    /// together, which is equivalent to sliding the whole camera rig in
    /// world space — a fixed world point then appears to move by the
    /// OPPOSITE of that shift, decomposed onto the camera's own right/up
    /// axes. Worked through by hand (not something I can verify by
    /// reading source the way an API signature can be) to make content
    /// follow the finger 1:1; if it ends up feeling backwards on device,
    /// it's a one-line sign flip on either term below, same as the
    /// orbit-drag sensitivity comment above.
    pub fn pan(&mut self, screen_dx: f32, screen_dy: f32, screen_width_px: f32, screen_height_px: f32) {
        if self.mode != CameraMode::LockedOrtho {
            return;
        }
        if screen_width_px <= 0.0 || screen_height_px <= 0.0 {
            return;
        }
        let aspect = screen_width_px / screen_height_px;
        let (half_width, half_height) = self.ortho_half_extents(aspect);
        let world_per_px_x = (half_width * 2.0) / screen_width_px;
        let world_per_px_y = (half_height * 2.0) / screen_height_px;

        let (right, up, _forward) = self.basis();
        self.pan_offset -= right * (screen_dx * world_per_px_x);
        self.pan_offset += up * (screen_dy * world_per_px_y);
    }

    /// Resets the camera to look directly at the canvas front-on —
    /// clears pan, snaps yaw/pitch back to zero, and forces LockedOrtho.
    /// "Focus on the canvas regardless of where the axis is" means
    /// actually facing it again, not just recentering while still looking
    /// sideways at it.
    pub fn focus_canvas(&mut self) {
        self.pan_offset = Vec3::ZERO;
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.mode = CameraMode::LockedOrtho;
    }

    /// Snaps to look directly down a cardinal axis — works in either
    /// mode, and deliberately bypasses `MAX_DRAG_PITCH`: a snap lands
    /// exactly on ±90° for true top/bottom views, which is safe here
    /// because `up_vector` special-cases exactly that angle.
    pub fn snap_to_axis(&mut self, axis: Axis) {
        let (yaw, pitch) = axis.yaw_pitch_degrees();
        self.yaw = yaw.to_radians();
        self.pitch = pitch.to_radians();
    }

    /// True when looking straight down -Z at the default front angle.
    pub fn is_front_view(&self) -> bool {
        self.yaw.abs() < 0.001 && self.pitch.abs() < 0.001
    }

    /// Half-width/half-height of the LockedOrtho frustum at the given
    /// aspect ratio — the same math `view_proj`'s LockedOrtho branch uses
    /// internally, exposed so a caller can map a front-view touch
    /// position into world space.
    pub fn ortho_half_extents(&self, aspect: f32) -> (f32, f32) {
        let half_height = self.radius * ORTHO_HALF_HEIGHT_FACTOR;
        (half_height * aspect, half_height)
    }
/// Returns the pan offset's X and Y world-space components — the 2D
    /// shift of the look-at target used to correct touch→canvas UV
    /// mapping when the canvas has been panned away from the origin.
    /// Only X/Y matter for the front-view mapping; Z is the depth axis
    /// and the canvas always sits at Z=0.
    pub fn pan_offset_xy(&self) -> (f32, f32) {
        (self.pan_offset.x, self.pan_offset.y)
        }
    /// Short human-readable camera state, for an on-screen readout.
    pub fn readout(&self) -> String {
        match self.mode {
            CameraMode::LockedOrtho => format!("Locked Ortho — zoom {:.2}", self.radius),
            CameraMode::FreeOrbit => format!(
                "Free Orbit — yaw {:.0}°  pitch {:.0}°  dist {:.2}",
                self.yaw.to_degrees(),
                self.pitch.to_degrees(),
                self.radius,
            ),
        }
    }

    /// Unit direction from the look-at target toward the eye.
    fn direction(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw)
    }

    fn eye(&self) -> Vec3 {
        self.direction() * self.radius + self.pan_offset
    }

    /// World "up" reference fed to `look_at_rh`. `Vec3::Y` everywhere
    /// except within ~1° of looking straight up/down — there, the view
    /// direction and `Y` become parallel, which makes `look_at_rh`'s
    /// internal cross product degenerate (NaN). Free-drag never reaches
    /// this (clamped to ±85°); only `snap_to_axis`'s exact ±90° top/
    /// bottom views do.
    fn up_vector(&self) -> Vec3 {
        if self.pitch > 89.0_f32.to_radians() {
            Vec3::NEG_Z
        } else if self.pitch < -89.0_f32.to_radians() {
            Vec3::Z
        } else {
            Vec3::Y
        }
    }

    /// `(right, up, forward)` camera-space basis for the current view —
    /// `forward` points from the eye toward the look-at target. Used by
    /// the gizmo to project world axes into screen space, and by `pan`
    /// to convert screen-pixel drag into a world-space offset. Same
    /// cross-product order `Mat4::look_at_rh` uses internally, so this
    /// basis matches what's actually on screen.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = -self.direction();
        let up_hint = self.up_vector();
        let right = forward.cross(up_hint).normalize();
        let up = right.cross(forward);
        (right, up, forward)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.pan_offset, self.up_vector());
        match self.mode {
            CameraMode::LockedOrtho => {
                let (half_width, half_height) = self.ortho_half_extents(aspect);
                let proj = Mat4::orthographic_rh(
                    -half_width, half_width, -half_height, half_height, 0.1, 100.0,
                );
                proj * view
            }
            CameraMode::FreeOrbit => {
                let proj = Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
                proj * view
            }
        }
    }
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl Camera for OrbitCamera {
    fn view_proj(&self, aspect: f32) -> Mat4 {
        self.view_proj(aspect)
    }
        }
