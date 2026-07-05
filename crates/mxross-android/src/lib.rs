// crates/mxross-android/src/lib.rs
//! MxRoss Canvas — Android entry point.

mod gizmo;
mod gpu;
mod ui;

use std::time::Duration;

use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};

use gpu::GpuState;

const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 30.0 / 255.0,
    g: 30.0 / 255.0,
    b: 40.0 / 255.0,
    a: 1.0,
};

/// Crash log stays in the app-private sandbox on purpose — unlike an
/// export, a crash log isn't something you want cluttering the Gallery,
/// and it's not something that needs MediaStore visibility at all.
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

/// Writes the exported PNG into the Gallery-visible
/// `Pictures/MxRoss/canvas.png` via MediaStore (see
/// mxross-android-media) — NOT the hidden `Android/data/...` sandbox
/// `crash.txt` uses. Returns a short human-readable result string either
/// way, for `AppUi`'s on-screen status line.
fn save_export(bytes: &[u8]) -> String {
    match mxross_android_media::save_png_to_pictures("canvas.png", "MxRoss", bytes) {
        Ok(()) => {
            log::info!("Exported canvas to Pictures/MxRoss/canvas.png");
            "Exported to Pictures/MxRoss/canvas.png".to_string()
        }
        Err(e) => {
            log::error!("failed to write exported PNG: {e}");
            format!("Export failed: {e}")
        }
    }
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    mxross_ffi::init_logger();
    install_panic_hook(&app);

    let mut gpu: Option<GpuState> = None;
    // Carries the painting across a TerminateWindow/InitWindow rebuild —
    // see GpuState::snapshot_canvas's doc comment for why this can't
    // just live inside GpuState itself.
    let mut canvas_snapshot: Option<(u32, u32, bool, Vec<u8>)> = None;

    loop {
        app.poll_events(Some(Duration::from_millis(16)), |event| {
            match event {
                PollEvent::Main(MainEvent::InitWindow { .. }) => {
                    if let Some(window) = app.native_window() {
                        match GpuState::new(window) {
                            Ok(mut state) => {
                                if let Some((width, height, pixel_art, pixels)) = canvas_snapshot.take() {
                                    state.restore_canvas(width, height, pixel_art, &pixels);
                                }
                                gpu = Some(state);
                            }
                            Err(e) => log::error!("wgpu setup failed: {e}"),
                        }
                    }
                }
                PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                    if let Some(state) = gpu.take() {
                        canvas_snapshot = state.snapshot_canvas();
                    }
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
            if let Some(bytes) = state.take_pending_export() {
                let status = save_export(&bytes);
                state.set_export_status(status);
            }
        }
    }
                 }
