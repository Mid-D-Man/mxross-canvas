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

/// Writes an exported PNG to `external_data_path()/exports/canvas.png`
/// (falling back to internal storage), the same location pattern
/// `crash.txt` uses — readable with a normal file manager at
/// `Android/data/com.midmanstudio.mxross/files/exports/canvas.png`.
/// Overwrites the same filename every time for now; multiple distinct
/// exports (timestamped names, or a proper save dialog) is a follow-up,
/// not needed for "does export actually work end to end."
fn save_export(app: &AndroidApp, bytes: &[u8]) {
    let Some(dir) = app.external_data_path().or_else(|| app.internal_data_path()) else {
        log::error!("no writable storage path available for export");
        return;
    };
    let path = dir.join("exports").join("canvas.png");
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::error!("failed to create export directory: {e}");
            return;
        }
    }
    match std::fs::write(&path, bytes) {
        Ok(()) => log::info!("Exported canvas to {}", path.display()),
        Err(e) => log::error!("failed to write exported PNG: {e}"),
    }
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    mxross_ffi::init_logger();
    install_panic_hook(&app);

    let mut gpu: Option<GpuState> = None;

    loop {
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
                save_export(&app, &bytes);
            }
        }
    }
    }
