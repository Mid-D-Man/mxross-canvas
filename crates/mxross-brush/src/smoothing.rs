// crates/mxross-brush/src/smoothing.rs
//! Rolling Catmull-Rom smoothing for a live stroke's raw input samples.
//! Touchscreen sampling is noisy/jittery at the pixel level even when
//! your hand is moving smoothly — this fits a curve through the last
//! few raw points instead of connecting them with straight segments.
//!
//! Needs 4 points (P0..P3) to evaluate the curve between P1 and P2 (P0
//! and P3 only shape the tangents) — which means a live stroke always
//! lags by one sample: the segment ending at the newest point isn't
//! drawable until the *next* point arrives to serve as its far tangent.
//! That's standard for rolling spline smoothing, not a bug; `flush`
//! handles the tail once the stroke ends (duplicating the last point as
//! its own missing tangent, the usual simple fix for an open curve end).
//!
//! Uniform Catmull-Rom specifically, not centripetal/chordal — simpler,
//! and good enough for touch input. `mid-math` has all three variants
//! via its `Interpolate` trait, but it's still under active refactor
//! (per project notes), so this stays a small self-contained
//! implementation rather than an early dependency on code that might
//! still move.

const SUBDIVISIONS: usize = 8;

pub struct StrokeSmoother {
    points: Vec<(f32, f32)>,
}

impl StrokeSmoother {
    pub fn new() -> Self {
        Self { points: Vec::new() }
    }

    pub fn reset(&mut self) {
        self.points.clear();
    }

    /// Feed a new raw sample. Returns the smoothed sub-points for the
    /// segment this completes, in stroke order — empty if there isn't
    /// enough history yet (the first 3 samples of a stroke).
    pub fn push(&mut self, point: (f32, f32)) -> Vec<(f32, f32)> {
        self.points.push(point);
        if self.points.len() < 4 {
            return Vec::new();
        }
        let segment = catmull_rom_segment(
            self.points[0], self.points[1], self.points[2], self.points[3], SUBDIVISIONS,
        );
        self.points.remove(0); // slide the window forward by one
        segment
    }

    /// Call once when the stroke ends to flush whatever segment is left
    /// un-emitted (the tail end always lags behind by design — see
    /// module doc comment).
    pub fn flush(&mut self) -> Vec<(f32, f32)> {
        let result = match self.points.len() {
            0 | 1 => Vec::new(),
            2 => vec![self.points[1]], // too short to curve-fit at all
            n => {
                let (p0, p1, p2) = (self.points[n - 3], self.points[n - 2], self.points[n - 1]);
                catmull_rom_segment(p0, p1, p2, p2, SUBDIVISIONS)
            }
        };
        self.points.clear();
        result
    }
}

impl Default for StrokeSmoother {
    fn default() -> Self {
        Self::new()
    }
}

fn catmull_rom_segment(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    subdivisions: usize,
) -> Vec<(f32, f32)> {
    (1..=subdivisions)
        .map(|i| catmull_rom_point(p0, p1, p2, p3, i as f32 / subdivisions as f32))
        .collect()
}

fn catmull_rom_point(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let t2 = t * t;
    let t3 = t2 * t;
    let axis = |a0: f32, a1: f32, a2: f32, a3: f32| -> f32 {
        0.5 * (2.0 * a1
            + (-a0 + a2) * t
            + (2.0 * a0 - 5.0 * a1 + 4.0 * a2 - a3) * t2
            + (-a0 + 3.0 * a1 - 3.0 * a2 + a3) * t3)
    };
    (axis(p0.0, p1.0, p2.0, p3.0), axis(p0.1, p1.1, p2.1, p3.1))
}
