//! MxRoss Canvas - Core Engine
//! Platform-agnostic drawing engine
//! Works on: Android, iOS, Desktop (macOS, Windows, Linux)

pub mod renderer;
pub mod canvas;
pub mod math;
pub mod utils;

pub use renderer::Renderer;
pub use canvas::{Canvas, CanvasPoint, Layer};

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn core_engine_initializes() {
        let _renderer = Renderer::new();
        assert!(true);
    }
    
    #[test]
    fn canvas_creates() {
        let canvas = Canvas::new(800, 600);
        assert_eq!(canvas.width, 800);
        assert_eq!(canvas.height, 600);
    }
    
    #[test]
    fn can_add_layers() {
        let mut canvas = Canvas::new(1024, 1024);
        canvas.add_layer("Layer 1");
        canvas.add_layer("Layer 2");
        assert_eq!(canvas.layers.len(), 3); // +1 for background
    }
}
