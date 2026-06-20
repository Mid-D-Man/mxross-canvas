//! Platform bridge — logging setup today, the C-ABI surface for host apps
//! eventually.
//!
//! NOTE: the original `utils::init_logger` referenced `android_logger`
//! without declaring it as a dependency. That would've failed the moment
//! this crate got cross-compiled for Android — fixed via the
//! `cfg(target_os = "android")` dependency above.

use std::time::Instant;

pub fn init_logger() {
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::Level::Debug)
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
