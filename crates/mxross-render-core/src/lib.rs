//! Shared rendering contract. Concrete backends (`mxross-render-cpu` today,
//! `mxross-render-gpu` later) implement this trait — same role
//! `msx-render-core::Renderer` plays for MSX.

use anyhow::Result;
use mxross_canvas_core::Canvas;

pub trait Renderer {
    fn render(&self, canvas: &Canvas) -> Result<()>;
    fn resize(&mut self, width: u32, height: u32);
    fn clear(&self, color: [f32; 4]);
}
