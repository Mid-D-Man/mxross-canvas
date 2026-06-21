use std::time::Duration;

use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use android_activity::input::InputEvent;
use ndk::hardware_buffer_format::HardwareBufferFormat;
use ndk::native_window::NativeWindow;

const BACKGROUND: [u8; 4]  = [30, 30, 40, 255];   // dark canvas, RGBA
const BRUSH_COLOR: [u8; 4] = [255, 90, 60, 255];  // dot wherever you touch
const BRUSH_RADIUS: i32    = 24;

#[no_mangle]
fn android_main(app: AndroidApp) {
    mxross_ffi::init_logger();

    let mut window: Option<NativeWindow> = None;
    // Last touch position in window pixel coordinates. None = nothing drawn yet.
    let mut touch: Option<(f32, f32)> = None;

    loop {
        // 16ms timeout instead of the original 0ms — gives a steady ~60fps
        // tick without spinning the CPU flat-out between frames.
        app.poll_events(Some(Duration::from_millis(16)), |event| {
            match event {
                PollEvent::Main(MainEvent::InitWindow { .. }) => {
                    if let Some(win) = app.native_window() {
                        // Force a known pixel format so the per-pixel math
                        // below can assume exactly 4 bytes per pixel.
                        // 0, 0 keeps the window's native size.
                        let _ = win.set_buffers_geometry(
                            0,
                            0,
                            Some(HardwareBufferFormat::R8G8B8A8_UNORM),
                        );
                        window = Some(win);
                    }
                }
                PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                    window = None;
                }
                _ => {}
            }
        });

        // Draining input every tick rather than waiting for
        // MainEvent::InputAvailable — simpler, correct, just not the most
        // efficient pattern. Fine for a "does this work" test.
        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let has_more = iter.next(|event| {
                    if let InputEvent::MotionEvent(motion) = event {
                        if let Some(pointer) = motion.pointers().next() {
                            touch = Some((pointer.x(), pointer.y()));
                        }
                    }
                    InputStatus::Unhandled
                });
                if !has_more {
                    break;
                }
            }
        }

        if let Some(win) = &window {
            draw_frame(win, touch);
        }
    }
}

fn draw_frame(window: &NativeWindow, touch: Option<(f32, f32)>) {
    let Ok(mut buffer) = window.lock(None) else { return };
    let Some(lines) = buffer.lines() else { return };

    for (row, line) in lines.enumerate() {
        for (col, pixel) in line.chunks_exact_mut(4).enumerate() {
            let inside_brush = touch.is_some_and(|(tx, ty)| {
                let dx = col as f32 - tx;
                let dy = row as f32 - ty;
                dx * dx + dy * dy <= (BRUSH_RADIUS * BRUSH_RADIUS) as f32
            });
            let color = if inside_brush { BRUSH_COLOR } else { BACKGROUND };
            pixel[0].write(color[0]);
            pixel[1].write(color[1]);
            pixel[2].write(color[2]);
            pixel[3].write(color[3]);
        }
    }
}
