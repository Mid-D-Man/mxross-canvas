//! Canvas and drawing data structures — no rendering logic here.
//! (Ported from the original core-engine `canvas` module.)

use mxross_math::Vec2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CanvasPoint {
    pub position: Vec2,
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub timestamp: f64,
}

impl CanvasPoint {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            position: Vec2::new(x, y),
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            timestamp: 0.0,
        }
    }

    pub fn with_pressure(mut self, pressure: f32) -> Self {
        self.pressure = pressure.clamp(0.0, 1.0);
        self
    }

    pub fn with_tilt(mut self, tilt_x: f32, tilt_y: f32) -> Self {
        self.tilt_x = tilt_x;
        self.tilt_y = tilt_y;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend_mode: BlendMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
        }
    }
}

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub active_layer: usize,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        log::info!("Creating canvas: {}x{}", width, height);
        Canvas {
            width,
            height,
            layers: vec![Layer::new("Background")],
            active_layer: 0,
        }
    }

    pub fn add_layer(&mut self, name: impl Into<String>) {
        self.layers.push(Layer::new(name));
        log::info!("Added layer: {}", self.layers.last().unwrap().name);
    }

    pub fn remove_layer(&mut self, index: usize) {
        if index < self.layers.len() && self.layers.len() > 1 {
            self.layers.remove(index);
            if self.active_layer >= self.layers.len() {
                self.active_layer = self.layers.len() - 1;
            }
        }
    }

    pub fn clear(&mut self) {
        log::info!("Clearing canvas");
    }

    pub fn draw_point(&mut self, point: &CanvasPoint) {
        log::trace!("Drawing point at {:?} with pressure {}", point.position, point.pressure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
