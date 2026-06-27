// crates/mxross-brush/src/hook.rs
//! The Lua plug-in seam. Defined as a plain Rust trait, deliberately
//! with zero scripting-engine knowledge — `BrushEngine` only ever calls
//! `on_dab` through this trait, so whatever `mlua`-backed implementation
//! eventually exists lives entirely on the other side of it. If that
//! implementation (or `mlua` itself, on Android specifically) ever turns
//! out to be a problem, the core engine is unaffected — it's already
//! running on `NoOpHook` and stays running on it.

#[derive(Clone, Copy)]
pub struct DabContext {
    /// Canvas UV (0..1) where this dab is about to land.
    pub position: (f32, f32),
    /// 0 for the first dab of the stroke, incrementing from there.
    pub dab_index: u32,
    /// Seconds since the stroke started.
    pub elapsed_secs: f32,
    /// The preset's un-overridden values, for a hook that wants to scale
    /// relative to the base rather than set an absolute value.
    pub base_radius_px: f32,
    pub base_color: [f32; 4],
}

/// What a hook can change about one dab. `None` means "use the preset's
/// value, unchanged." `extra_dabs` is for particle/scatter-style brushes
/// that want to emit more than one dab per spacing step — each entry is
/// stamped at the base radius/color, at that canvas UV position.
#[derive(Default)]
pub struct DabOverride {
    pub radius_px: Option<f32>,
    pub color: Option<[f32; 4]>,
    pub extra_dabs: Vec<(f32, f32)>,
}

pub trait BrushHook {
    fn on_dab(&mut self, ctx: &DabContext) -> DabOverride;
}

/// The default — every preset gets this until something actually
/// requests scripted behavior.
pub struct NoOpHook;

impl BrushHook for NoOpHook {
    fn on_dab(&mut self, _ctx: &DabContext) -> DabOverride {
        DabOverride::default()
    }
}
