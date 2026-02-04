//! Utility functions

use std::time::Instant;

pub fn init_logger() {
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_min_level(log::Level::Debug)
                .with_tag("MxRossCanvas"),
        );
    }
    
    #[cfg(not(target_os = "android"))]
    {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    }
}

pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }
    
    pub fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}
