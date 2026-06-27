// crates/mxross-brush/src/lib.rs
//! MxRoss Canvas brush engine.
//!
//! Three-layer plan (mirrors Krita's paintop/sensor/Python split):
//!   - DixScript (.mdix, `preset.rs`): static preset data — brush kind,
//!     base parameter values. Resolved entirely at compile time via
//!     DixScript's QuickFuncs, so loading a preset at runtime is just
//!     flat field reads. NOT wired to `.mdix` loading yet — that's
//!     blocked on a Cargo.toml fix on the DixScript-Rust side (reqwest's
//!     default TLS backend), being handled separately.
//!   - Rust (`engine.rs`, `smoothing.rs`): the actual per-dab decision
//!     pipeline — smoothing, spacing, hook dispatch. Always native,
//!     never scripted, same reason Krita's paintops are compiled C++,
//!     not Python. Doesn't touch a GPU; `mxross-android`'s `canvas.rs`
//!     takes the `DabPlan`s this produces and does the actual drawing.
//!   - Lua (`hook.rs`'s `BrushHook` trait): an optional per-dab hook for
//!     brushes that need real procedural logic beyond a static preset —
//!     same role Python automation plays in Krita. The trait is defined
//!     here; an actual `mlua`-backed implementation is a separate
//!     concern (and its own Android-cross-compile question to verify),
//!     deliberately isolated behind this trait so it can't destabilize
//!     the core engine.

pub mod engine;
pub mod hook;
pub mod preset;
pub mod smoothing;

pub use engine::{BrushEngine, DabPlan};
pub use hook::{BrushHook, DabContext, DabOverride, NoOpHook};
pub use preset::BrushPreset;
pub use smoothing::StrokeSmoother;
