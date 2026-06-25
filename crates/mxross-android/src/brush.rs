// crates/mxross-android/src/brush.rs
//! Brush settings shared across brush kinds, plus the seam where a
//! second kind plugs in later.
//!
//! Only `Surface2D` exists right now. The planned `Mesh3D` counterpart
//! (texture-paint/sculpt-style painting directly onto a mesh, from any
//! camera angle, via a real ray-surface hit test instead of a flat-plane
//! unproject) is named in this doc comment rather than as a real enum
//! variant — there's no mesh to paint onto yet, so an unimplemented
//! variant would just be dead weight. Add it for real once there's
//! actual geometry to hit-test against.
//!
//! These fields are the minimal "already-resolved" subset of what a
//! future `.mdix` brush preset compiles down to (DixScript handles the
//! preset data — paintop choice, base option values, sensor->curve
//! mappings; this struct is the flat runtime shape that data eventually
//! fills in). Not wired to DixScript yet — just shaped so it can be.

#[derive(Clone, Copy)]
pub struct BrushSettings {
    pub radius_px: f32,
    pub color: [f32; 4],
    /// Distance between dabs along a stroke, as a fraction of
    /// `radius_px` — Krita/Photoshop/Procreate all call this exact
    /// concept "spacing". Smaller = smoother/denser line, more stamp
    /// calls per stroke. 0.25 is a reasonable starting point, not a
    /// measured-correct value.
    pub spacing: f32,
}

impl BrushSettings {
    pub fn default_ink() -> Self {
        Self {
            radius_px: 16.0,
            color: [0.08, 0.08, 0.1, 1.0], // near-black
            spacing: 0.25,
        }
    }
}
