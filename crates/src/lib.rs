//! Reserved slot for the `.kriter` project/document format.
//!
//! Container shape not decided yet. Candidates on the table:
//!   - single binary file — DixScript-described manifest (layer tree, blend
//!     modes, vector stroke paths) + per-layer MPX-encoded raster payloads,
//!     mirroring how `msx-binary` works.
//!   - zip/directory bundle, closer to `.kra` / `.ora`.
//!
//! Intentionally empty — this crate exists so the workspace dependency
//! graph is ready the moment the format is settled.
