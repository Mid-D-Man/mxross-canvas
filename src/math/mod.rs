//! Math utilities

pub use glam::*;

pub type Vec2 = glam::Vec2;
pub type Vec3 = glam::Vec3;
pub type Vec4 = glam::Vec4;
pub type Mat4 = glam::Mat4;

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    value.max(min).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_lerp() {
        assert_eq!(lerp(0.0, 10.0, 0.5), 5.0);
    }
    
    #[test]
    fn test_clamp() {
        assert_eq!(clamp(15.0, 0.0, 10.0), 10.0);
    }
}
