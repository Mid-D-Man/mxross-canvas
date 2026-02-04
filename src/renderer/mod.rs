//! Core renderer (platform-agnostic)

use anyhow::Result;

pub struct Renderer {
    pub initialized: bool,
}

impl Renderer {
    pub fn new() -> Self {
        log::info!("MxRoss Canvas: Core renderer initialized");
        Renderer { initialized: true }
    }
    
    pub fn render(&self) -> Result<()> {
        // Rendering implementation will go here
        Ok(())
    }
    
    pub fn resize(&mut self, width: u32, height: u32) {
        log::info!("Canvas resized: {}x{}", width, height);
    }
    
    pub fn clear(&self, color: [f32; 4]) {
        log::debug!("Clearing with color: {:?}", color);
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}
