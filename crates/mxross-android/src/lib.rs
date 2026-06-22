// crates/mxross-android/src/lib.rs
//! MxRoss Canvas — Android entry point.
//!
//! Real GPU path: Instance -> Surface -> Adapter -> Device, one
//! depth-tested render pass per frame drawing a hardcoded test cube. See
//! gpu.rs for the wgpu setup and test_cube.rs for the test geometry.

mod gpu;
mod test_cube;

use std::time::Duration;

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

        // Draining input every tick so events don't pile up. Not wired to
        // anything yet — this is where camera orbit/pan/zoom input lands
        // once the free 3D viewport replaces this test.
        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let has_more = iter.next(|_event| InputStatus::Unhandled);
                if !has_more {
                    break;
                }
            }
        }

        if let Some(state) = &gpu {
            state.render(BACKGROUND);
        }
    }
            }
