// crates/mxross-android/src/lib.rs
//! MxRoss Canvas — Android entry point.

mod brush;
mod camera;
mod canvas;
mod gizmo;
mod gpu;
mod ui;

use std::time::Duration;

use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};

use gpu::GpuState;

/// Dark canvas background — visible in the margin around the paint
/// canvas plane (the canvas itself is the white quad).
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 30.0 / 255.0,
    g: 30.0 / 255.0,
    b: 40.0 / 255.0,
    a: 1.0,
};

/// Writes any future panic to a plain text file instead of (only) the
/// Android log, since logcat isn't always reachable from a phone-only
/// workflow. Prefers external storage — on most devices that's
/// `/storage/emulated/0/Android/data/com.midmanstudio.mxross/files/crash.txt`,
/// browsable with a normal file manager.
fn install_panic_hook(app: &AndroidApp) {
    let crash_path = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .map(|dir| dir.join("crash.txt"));

    if let Some(path) = crash_path {
        std::panic::set_hook(Box::new(move |info| {
            let _ = std::fs::write(&path, format!("{info}\n"));
            log::error!("PANIC: {info}");
        }));
    }
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    mxross_ffi::init_logger();
    install_panic_hook(&app);

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

        let pixels_per_point = app.config().density().unwrap_or(160) as f32 / 160.0;

        // Down (first finger) and PointerDown (an additional finger) are
        // handled separately on purpose — see GpuState::second_touch_down's
        // doc comment for why collapsing them caused a stray-dab bug.
        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let has_more = iter.next(|event| {
                    if let InputEvent::MotionEvent(motion) = event {
                        if let Some(state) = gpu.as_mut() {
                            match motion.action() {
                                MotionAction::Down => {
                                    if let Some(p) = motion.pointers().next() {
                                        state.touch_down(p.x(), p.y(), pixels_per_point);
                                    }
                                }
                                MotionAction::PointerDown => {
                                    state.second_touch_down();
                                }
                                MotionAction::Move => {
                                    let pointers: Vec<(f32, f32)> =
                                        motion.pointers().map(|p| (p.x(), p.y())).collect();
                                    state.touch_move(&pointers, pixels_per_point);
                                }
                                MotionAction::Up | MotionAction::PointerUp | MotionAction::Cancel => {
                                    if let Some(p) = motion.pointers().next() {
                                        state.touch_up(p.x(), p.y(), pixels_per_point);
                                    }
                                }
                                _ => {}
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
