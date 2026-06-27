// crates/mxross-brush/src/preset.rs
//! Brush preset data — the static, already-resolved subset of what a
//! `.mdix` brush preset will eventually compile down to. Mirrors what
//! Krita's `.kpp` files store: base option values (size, spacing) and,
//! eventually, sensor->curve mappings. Color is deliberately included
//! here for now (a real Krita-style setup keeps color separate, driven
//! by a color picker rather than the brush preset) — revisit once
//! there's an actual color UI; baking it into the preset is the simpler
//! thing to ship first.
//!
//! `from_mdix_str` isn't here yet — that needs the `dixscript` crate as
//! a real dependency, which is blocked on a Cargo.toml fix upstream
//! (reqwest's default TLS backend pulls in native-tls/OpenSSL, a real
//! Android cross-compile risk). Once that's resolved, this is where
//! loading from `.mdix` source text lands.

#[derive(Clone, Copy)]
pub struct BrushPreset {
    pub radius_px: f32,
    pub color: [f32; 4],
    /// Distance between dabs along a stroke, as a fraction of
    /// `radius_px` — Krita/Photoshop/Procreate all call this exact
    /// concept "spacing". Smaller = smoother/denser line.
    pub spacing: f32,
}

impl BrushPreset {
    pub fn default_ink() -> Self {
        Self {
            radius_px: 16.0,
            color: [0.08, 0.08, 0.1, 1.0], // near-black
            spacing: 0.25,
        }
    }
  }
