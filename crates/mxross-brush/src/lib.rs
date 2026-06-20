//! Reserved slot for the brush engine.
//!
//! Planned split (not implemented yet):
//!   - DixScript: static preset definitions (size, spacing, jitter,
//!     dynamics curve shapes) — compiled once, like MSX's QuickFuncs.
//!   - Lua (mlua/rlua): runtime scripting hook for per-stroke dynamic
//!     behavior and user macros.
//!   - Rust: the actual per-pixel stamping/compositing hot path.
