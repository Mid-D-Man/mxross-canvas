// crates/mxross-android/src/camera.rs
//! Touch-driven camera for the test cube — two modes, matching the
//! decided MxRoss Canvas viewport architecture:
//!   - `LockedOrtho` (default): flat orthographic view, no drag input,
//!     pinch-to-zoom only.
//!   - `FreeOrbit`: perspective orbit, single-finger drag + pinch-zoom.
//! Toggled via the egui button in ui.rs. Still part of the throwaway
//! test scene, not the real canvas viewport — that comes later, once
//! there's real content to look at and mid-math is wired in for the
//! projection/picking math.

use mxross_math::{Mat4, Vec3};

/// Pixels-to-radians drag sensitivity. If orbiting feels backwards on
/// device, flip the sign on the corresponding line in `handle_drag` —
/// the "natural" drag direction is a matter of feel, not something to
/// derive from first principles.
const ORBIT_SENSITIVITY: f32 = 0.005;

/// Keeps pitch away from the poles — at exactly ±90° `yaw` stops doing
/// anything (gimbal lock) and the look-at "up" vector starts to wobble as
/// you approach it.
const MAX_PITCH: f32 = 85.0 / 180.0 * std::f32::consts::PI;

/// Pinch-zoom clamp on `radius` — arbitrary, easy to retune. Also bounds
/// how close/far the LockedOrtho half-height can get, since both modes
/// share this one knob.
const MIN_RADIUS: f32 = 1.0;
const MAX_RADIUS: f32 = 20.0;

/// `LockedOrtho`'s half-height as a fraction of `radius` — chosen so the
/// default radius (4.0) reproduces the original hardcoded half-height
/// (2.5) exactly, while still tracking pinch-zoom the same way
/// FreeOrbit's distance does.
const ORTHO_HALF_HEIGHT_FACTOR: f32 = 0.625;

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
            yaw: 45.0_f32.to_radians(),
            pitch: 30.0_f32.to_radians(),
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
        self.pitch = (self.pitch - dy * ORBIT_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// `factor` is `current_pinch_distance / previous_pinch_distance` —
    /// >1.0 means fingers spread apart (zoom in / move closer), <1.0
    /// means fingers pinched together (zoom out). Works in both camera
    /// modes since both `view_proj` branches read `radius`.
    pub fn zoom(&mut self, factor: f32) {
        if factor <= 0.0 || !factor.is_finite() {
            return;
        }
        self.radius = (self.radius / factor).clamp(MIN_RADIUS, MAX_RADIUS);
    }

    /// Short human-readable camera state, for the on-screen readout.
    /// Shown as yaw/pitch/distance (the camera's actual internal state)
    /// rather than raw XYZ — more meaningful for an orbit camera, but
    /// say so if you'd rather see Cartesian eye coordinates instead.
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

    fn eye(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(
            self.radius * cos_pitch * sin_yaw,
            self.radius * sin_pitch,
            self.radius * cos_pitch * cos_yaw,
        )
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        match self.mode {
            CameraMode::LockedOrtho => {
                let half_height = self.radius * ORTHO_HALF_HEIGHT_FACTOR;
                let half_width = half_height * aspect;
                let proj = Mat4::orthographic_rh(
                    -half_width, half_width, -half_height, half_height, 0.1, 100.0,
                );
                let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
                proj * view
            }
            CameraMode::FreeOrbit => {
                let proj = Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
                let view = Mat4::look_at_rh(self.eye(), Vec3::ZERO, Vec3::Y);
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
