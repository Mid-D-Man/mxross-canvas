// crates/mxross-android/src/lib.rs
//! MxRoss Canvas — Android entry point.
//!
//! Real GPU path: Instance -> Surface -> Adapter -> Device, one
//! depth-tested render pass per frame drawing a hardcoded test cube from
//! a touch-driven camera, plus an egui UI overlay (the locked-ortho/
//! free-orbit toggle). See gpu.rs (wgpu setup), test_cube.rs (test
//! geometry), camera.rs (camera math), ui.rs (egui integration).

mod camera;
mod gpu;
mod test_cube;
mod ui;

use std::time::Duration;

use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};

use gpu::GpuState;

/// Dark canvas background — same color the original software-pixel test
/// used.
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 30.0 / 255.0,
    g: 30.0 / 255.0,
    b: 40.0 / 255.0,
    a: 1.0,
};

#[no_mangle]
fn android_main(app: AndroidApp) {
    mxross_ffi::init_logger();

    let mut gpu: Option<GpuState> = None;

    loop {
        // 16ms timeout instead of 0ms — steady ~60fps tick without
        // spinning the CPU flat-out between frames.
        app.poll_events(Some(Duration::from_millis(16)), |event| {
            match event {
                PollEvent::Main(MainEvent::InitWindow { .. }) => {
                    if let Some(window) = app.native_window() {
                        match GpuState::new(window) {
                            Ok(state) => gpu = Some(state),
                            Err(e) => log::error!("wgpu setup failed: {e}"),
                        }
                    }
                }
                PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                    gpu = None;
                }
                PollEvent::Main(MainEvent::WindowResized { .. }) => {
                    if let (Some(window), Some(state)) = (app.native_window(), gpu.as_mut()) {
                        let width = window.width().max(1) as u32;
                        let height = window.height().max(1) as u32;
                        state.resize(width, height);
                    }
                }
                _ => {}
            }
        });

        // Density bucket / 160 — Android's standard DPI-scale convention
        // (160 = mdpi = 1.0x). Cheap enough to just recompute every tick
        // rather than caching and tracking ConfigChanged separately.
        let pixels_per_point = app.config().density().unwrap_or(160) as f32 / 160.0;

        // Single-finger touch -> both the orbit camera and egui. Only
        // ever looks at the first pointer — no multi-touch handling yet.
        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let has_more = iter.next(|event| {
                    if let InputEvent::MotionEvent(motion) = event {
                        if let Some(pointer) = motion.pointers().next() {
                            let (x, y) = (pointer.x(), pointer.y());
                            if let Some(state) = gpu.as_mut() {
                                match motion.action() {
                                    MotionAction::Down | MotionAction::PointerDown => {
                                        state.touch_down(x, y, pixels_per_point);
                                    }
                                    MotionAction::Move => {
                                        state.touch_move(x, y, pixels_per_point);
                                    }
                                    MotionAction::Up | MotionAction::PointerUp | MotionAction::Cancel => {
                                        state.touch_up(x, y, pixels_per_point);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    InputStatus::Unhandled
                });
                if !has_more {
                    break;
                }
            }
        }

        if let Some(state) = gpu.as_mut() {
            state.render(BACKGROUND, pixels_per_point);
        }
    }
                                        }
