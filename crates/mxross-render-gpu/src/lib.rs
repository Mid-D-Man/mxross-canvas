// crates/mxross-render-gpu/src/lib.rs
//! GPU rendering for MxRoss Canvas. Knows wgpu; knows nothing about
//! brushes, strokes, presets, or windows/Surface — mirrors how
//! msx-render-gpu only renders, leaving Surface/window concerns to the
//! app (mxross-android, for now).
//!
//! `Dab` is this crate's own minimal "what to draw" type — deliberately
//! NOT `mxross-brush`'s `DabPlan`. The two crates don't depend on each
//! other; `mxross-android`'s gpu.rs is the only place that knows both
//! exist, converting one into the other.

pub mod canvas;

pub use canvas::{Dab, PaintCanvas};
