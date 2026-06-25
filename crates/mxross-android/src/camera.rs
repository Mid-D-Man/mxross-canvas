// crates/mxross-android/src/camera.rs
//! Touch-driven camera for the paint canvas — two modes, matching the
//! decided MxRoss Canvas viewport architecture:
//!   - `LockedOrtho` (default): flat orthographic view, no drag input,
//!     pinch-to-zoom and axis-gizmo snapping. Painting (canvas.rs) only
//!     works here, and only in the front-facing orientation specifically
//!     — see `is_front_view`.
//!   - `FreeOrbit`: perspective orbit, single-finger drag + pinch-zoom +
//!     axis-gizmo snapping. No painting in this mode yet.
//! Both modes share one yaw/pitch/radius state — "locked" only means
//! "no free dragging", not "frozen direction"; clicking a gizmo ball (see
//! gizmo.rs) re-points either mode at a cardinal axis, exactly like
//! Blender's numpad views.

use mxross_math::{Mat4, Vec3};

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
/// since both modes share this one knob.
const MIN_RADIUS: f32 = 1.0;
const MAX_RADIUS: f32 = 20.0;

/// `LockedOrtho`'s half-height as a fraction of `radius` — chosen so the
/// default radius (4.0) gives a half-height of 2.5, slightly larger than
/// the paint canvas's own half-size (2.0) so the canvas doesn't fill the
/// entire screen at default zoom (leaves room for the UI corners).
const ORTHO_HALF_HEIGHT_FACTOR: f32 = 0.625;

/// A cardinal world axis, as offered by the gizmo (gizmo.rs).
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
}

impl OrbitCamera {
    pub fn new() -> Self {
        Self {
            // Front view by default — the only orientation painting
            // currently understands (see `is_front_view`).
            yaw: 0.0,
            pitch: 0.0,
            radius: 4.0,
            mode: CameraMode::LockedOrtho,
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

    /// Snaps to look directly down a cardinal axis — works in either
    /// mode, and deliberately bypasses `MAX_DRAG_PITCH`: a snap lands
    /// exactly on ±90° for true top/bottom views, which is safe here
    /// because `up_vector` special-cases exactly that angle.
    pub fn snap_to_axis(&mut self, axis: Axis) {
        let (yaw, pitch) = axis.yaw_pitch_degrees();
        self.yaw = yaw.to_radians();
        self.pitch = pitch.to_radians();
    }

    /// True when looking straight down -Z at the default front angle —
    /// the only orientation `canvas.rs` currently knows how to map touch
    /// input through (a simple orthographic unproject, not a general
    /// ray-plane intersection). Snapping to Top/Side/Back via the gizmo
    /// leaves `LockedOrtho` active but makes this false, which is what
    /// currently disables painting from those angles.
    pub fn is_front_view(&self) -> bool {
        self.yaw.abs() < 0.001 && self.pitch.abs() < 0.001
    }

    /// Half-width/half-height of the LockedOrtho frustum at the given
    /// aspect ratio — the same math `view_proj`'s LockedOrtho branch uses
    /// internally, exposed so canvas.rs can map a front-view touch
    /// position into world space.
    pub fn ortho_half_extents(&self, aspect: f32) -> (f32, f32) {
        let half_height = self.radius * ORTHO_HALF_HEIGHT_FACTOR;
        (half_height * aspect, half_height)
    }

    /// Short human-readable camera state, for the on-screen readout.
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
        self.direction() * self.radius
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
    /// the gizmo to project world axes into screen space. Same
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
        let view = Mat4::look_at_rh(self.eye(), Vec3::ZERO, self.up_vector());
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
