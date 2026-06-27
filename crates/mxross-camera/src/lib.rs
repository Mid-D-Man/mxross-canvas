// crates/mxross-camera/src/lib.rs
//! MxRoss Canvas camera.
//!
//! Pure state/math — no wgpu, no egui, nothing rendering- or
//! UI-framework-specific. `mxross-render-gpu` only ever needs whatever a
//! camera produces via `Camera::view_proj`; `mxross-android`'s
//! `gizmo.rs`/`ui.rs` read `OrbitCamera`-specific state (mode, basis,
//! axis snapping) directly, since those are inherently orbit-camera
//! concepts, not things every future camera kind needs to support.
//!
//! `Camera` is deliberately minimal — just `view_proj` — so a future
//! second camera kind (whatever "advanced functionality" turns out to
//! mean — a scripted/animated camera for playback, a free fly camera,
//! anything else) can exist alongside `OrbitCamera` without any consumer
//! needing to change. Axis-snapping, "is this the front view", and
//! similar stay as inherent methods on `OrbitCamera` rather than being
//! forced into the shared trait — they don't generalize to every
//! possible camera kind, and a trait that pretends they do would just be
//! leaky.

mod orbit;

pub use orbit::{Axis, CameraMode, OrbitCamera};

use mxross_math::Mat4;

/// The one thing every camera kind unavoidably needs to provide.
pub trait Camera {
    fn view_proj(&self, aspect: f32) -> Mat4;
  }
