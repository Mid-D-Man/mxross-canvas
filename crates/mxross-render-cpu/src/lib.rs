//! CPU renderer — currently a stub, ported as-is from the original
//! core-engine `renderer` module. Real rasterization work lands here.

use anyhow::Result;
use mxross_canvas_core::Canvas;
use mxross_render_core::Renderer;

pub struct CpuRenderer {
    pub initialized: bool,
}

impl CpuRenderer {
    pub fn new() -> Self {
        log::info!("MxRoss Canvas: CPU renderer initialized");
        CpuRenderer { initialized: true }
    }
}

impl Default for CpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for CpuRenderer {
    fn render(&self, _canvas: &Canvas) -> Result<()> {
        // Rasterization implementation goes here.
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        log::info!("Canvas resized: {}x{}", width, height);
    }

    fn clear(&self, color: [f32; 4]) {
        log::debug!("Clearing with color: {:?}", color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_initializes() {
        let renderer = CpuRenderer::new();
        assert!(renderer.initialized);
    }
      }
