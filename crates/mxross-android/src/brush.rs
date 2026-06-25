// crates/mxross-android/src/brush.rs
//! Brush settings shared across brush kinds, plus the seam where a
//! second kind plugs in later.
//!
//! Only `Surface2D` exists right now. The planned `Mesh3D` counterpart
//! (texture-paint/sculpt-style painting directly onto a mesh, from any
//! camera angle, via a real ray-surface hit test instead of a flat-plane
//! unproject) is named in this doc comment rather than as a real enum
//! variant — there's no mesh to paint onto yet, so an unimplemented
//! variant would just be dead weight forcing `#[allow(dead_code)]` or
//! non-exhaustive matches everywhere for no present benefit. Add it for
//! real once there's actual geometry to hit-test against.

#[derive(Clone, Copy)]
pub struct BrushSettings {
    pub radius_px: f32,
    pub color: [f32; 4],
}

impl BrushSettings {
    pub fn default_ink() -> Self {
        Self {
            radius_px: 16.0,
            color: [0.08, 0.08, 0.1, 1.0], // near-black
        }
    }
  }
