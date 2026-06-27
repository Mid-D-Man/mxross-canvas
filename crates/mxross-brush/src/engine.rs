// crates/mxross-brush/src/engine.rs
//! Ties preset + smoothing + spacing + hook together into one
//! per-stroke decision pipeline. This is the entire "what should this
//! stroke do" answer — `mxross-android`'s `canvas.rs` just renders
//! whatever `DabPlan`s come out of it and has no brush logic of its own.
//!
//! Distance math (for spacing) happens in `canvas_size_px` units — a
//! plain dimension passed in at construction, not a wgpu type. That's
//! what keeps this crate free of any rendering-API knowledge while still
//! getting "spacing" to mean the same thing it means in Krita/Photoshop
//! (a fraction of brush radius, in real pixels) rather than an abstract
//! fraction of the 0..1 UV space, which wouldn't track brush size
//! correctly as the canvas resolution changes.

use std::time::Instant;

use crate::hook::{BrushHook, DabContext, NoOpHook};
use crate::preset::BrushPreset;
use crate::smoothing::StrokeSmoother;

pub struct DabPlan {
    pub position: (f32, f32), // canvas UV
    pub radius_px: f32,
    pub color: [f32; 4],
}

pub struct BrushEngine {
    preset: BrushPreset,
    hook: Box<dyn BrushHook>,
    smoother: StrokeSmoother,
    canvas_size_px: f32,
    last_stamp: Option<(f32, f32)>,
    dab_index: u32,
    stroke_start: Option<Instant>,
}

impl BrushEngine {
    pub fn new(preset: BrushPreset, canvas_size_px: f32) -> Self {
        Self {
            preset,
            hook: Box::new(NoOpHook),
            smoother: StrokeSmoother::new(),
            canvas_size_px,
            last_stamp: None,
            dab_index: 0,
            stroke_start: None,
        }
    }

    pub fn set_hook(&mut self, hook: Box<dyn BrushHook>) {
        self.hook = hook;
    }

    pub fn preset(&self) -> &BrushPreset {
        &self.preset
    }

    pub fn preset_mut(&mut self) -> &mut BrushPreset {
        &mut self.preset
    }

    /// Call on touch-down. Stamps once at `point` with no smoothing —
    /// there's no prior history to fit a curve through yet — and starts
    /// tracking a fresh stroke.
    pub fn start_stroke(&mut self, point: (f32, f32)) -> Vec<DabPlan> {
        self.smoother.reset();
        self.last_stamp = None;
        self.dab_index = 0;
        self.stroke_start = Some(Instant::now());
        self.smoother.push(point); // seeds history; return value unused
        self.emit_at(point)
    }

    /// Call on touch-move with a new raw sample.
    pub fn push_point(&mut self, point: (f32, f32)) -> Vec<DabPlan> {
        let smoothed = self.smoother.push(point);
        self.stamp_along(smoothed)
    }

    /// Call on touch-up to flush the stroke's tail end.
    pub fn end_stroke(&mut self) -> Vec<DabPlan> {
        let tail = self.smoother.flush();
        let plans = self.stamp_along(tail);
        self.last_stamp = None;
        self.stroke_start = None;
        plans
    }

    /// Call when a stroke is interrupted (e.g. a second finger landing
    /// to start a pinch) rather than ended normally — discards history
    /// without emitting a tail flush, so the interruption doesn't leave
    /// a stray connecting dab.
    pub fn cancel_stroke(&mut self) {
        self.smoother.reset();
        self.last_stamp = None;
        self.stroke_start = None;
    }

    fn stamp_along(&mut self, points: Vec<(f32, f32)>) -> Vec<DabPlan> {
        let mut plans = Vec::new();
        for target in points {
            plans.extend(self.walk_spacing(target));
        }
        plans
    }

    /// Walks evenly-spaced steps from `last_stamp` to `target` (spacing
    /// computed from the *base* preset radius, not any hook-overridden
    /// value, to avoid a shrinking-radius hook collapsing spacing toward
    /// zero), emitting one dab per step.
    fn walk_spacing(&mut self, target: (f32, f32)) -> Vec<DabPlan> {
        let Some(mut cursor) = self.last_stamp else {
            return self.emit_at(target);
        };

        let spacing_px = (self.preset.radius_px * self.preset.spacing).max(1.0);
        let mut plans = Vec::new();
        loop {
            let dx = (target.0 - cursor.0) * self.canvas_size_px;
            let dy = (target.1 - cursor.1) * self.canvas_size_px;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < spacing_px {
                self.last_stamp = Some(cursor);
                return plans;
            }
            let t = spacing_px / dist;
            cursor = (cursor.0 + (target.0 - cursor.0) * t, cursor.1 + (target.1 - cursor.1) * t);
            plans.extend(self.emit_at(cursor));
        }
    }

    fn emit_at(&mut self, position: (f32, f32)) -> Vec<DabPlan> {
        let ctx = DabContext {
            position,
            dab_index: self.dab_index,
            elapsed_secs: self.stroke_start.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0),
            base_radius_px: self.preset.radius_px,
            base_color: self.preset.color,
        };
        let over = self.hook.on_dab(&ctx);

        self.dab_index += 1;
        self.last_stamp = Some(position);

        let mut plans = vec![DabPlan {
            position,
            radius_px: over.radius_px.unwrap_or(self.preset.radius_px),
            color: over.color.unwrap_or(self.preset.color),
        }];
        plans.extend(over.extra_dabs.into_iter().map(|pos| DabPlan {
            position: pos,
            radius_px: self.preset.radius_px,
            color: self.preset.color,
        }));
        plans
    }
}
