// crates/mxross-android/src/gizmo.rs
//! Blender-style axis gizmo — six colored balls (±X/±Y/±Z) positioned by
//! projecting world axis directions through the camera's current
//! rotation. Clicking one snaps the camera to look directly down that
//! axis. Works in both camera modes — they share one yaw/pitch state,
//! see mxross-camera.
//!
//! Pure `egui::Painter` drawing, not a real 3D-rendered widget — this is
//! the standard trick every editor gizmo like this uses: take the
//! camera's basis vectors only (no projection matrix, no perspective),
//! dot each world axis against them to get a 2D screen-plane offset, and
//! draw a flat circle there. Depth (`axis.dot(forward)`) only affects
//! draw order and the filled-vs-outline styling, not actual perspective.

use mxross_math::Vec3;

use mxross_camera::Axis;

const SIZE: f32 = 120.0;
const RADIUS: f32 = 45.0;
const BALL_RADIUS: f32 = 11.0;

const AXES: [(Axis, &str, egui::Color32); 6] = [
    (Axis::PosX, "X", egui::Color32::from_rgb(226, 78, 78)),
    (Axis::NegX, "-X", egui::Color32::from_rgb(226, 78, 78)),
    (Axis::PosY, "Y", egui::Color32::from_rgb(96, 200, 96)),
    (Axis::NegY, "-Y", egui::Color32::from_rgb(96, 200, 96)),
    (Axis::PosZ, "Z", egui::Color32::from_rgb(86, 140, 230)),
    (Axis::NegZ, "-Z", egui::Color32::from_rgb(86, 140, 230)),
];

/// Draws the gizmo and returns the axis that was clicked this frame, if
/// any. `basis` is `(right, up, forward)` for the current camera view —
/// see `OrbitCamera::basis`.
pub fn show(ui: &mut egui::Ui, basis: (Vec3, Vec3, Vec3)) -> Option<Axis> {
    let (right, up, forward) = basis;
    let (response, painter) = ui.allocate_painter(egui::vec2(SIZE, SIZE), egui::Sense::click());
    let center = response.rect.center();

    // (axis, screen position, color, label, facing_camera)
    let mut balls: Vec<(Axis, egui::Pos2, egui::Color32, &str, bool)> = AXES
        .iter()
        .map(|&(axis, label, color)| {
            let dir = axis.direction();
            let offset = egui::vec2(dir.dot(right), -dir.dot(up)) * RADIUS;
            let facing_camera = dir.dot(forward) < 0.0;
            (axis, center + offset, color, label, facing_camera)
        })
        .collect();
    // Far balls first, near balls last, so near ones draw (and win
    // hit-testing ties) on top.
    balls.sort_by_key(|&(_, _, _, _, facing_camera)| facing_camera);

    for &(_, pos, color, label, facing_camera) in &balls {
        let fill = if facing_camera { color } else { egui::Color32::TRANSPARENT };
        painter.circle(pos, BALL_RADIUS, fill, egui::Stroke::new(1.5, color));
        let text_color = if facing_camera { egui::Color32::WHITE } else { color };
        painter.text(pos, egui::Align2::CENTER_CENTER, label, egui::FontId::monospace(11.0), text_color);
    }

    if !response.clicked() {
        return None;
    }
    let click_pos = response.interact_pointer_pos()?;

    balls
        .iter()
        .filter(|&&(_, pos, ..)| pos.distance(click_pos) <= BALL_RADIUS)
        .min_by(|a, b| {
            a.1.distance(click_pos)
                .partial_cmp(&b.1.distance(click_pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|&(axis, ..)| axis)
    }
