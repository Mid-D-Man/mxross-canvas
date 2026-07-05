// Pixel-art variant of canvas_stamp.wgsl — identical vertex stage, but
// the fragment stage has a hard cutoff instead of a smoothstep falloff.
// A soft edge is exactly what "blurry" means for pixel art: any
// antialiasing at the dab boundary introduces intermediate alpha
// values, which read as a fuzzy border once combined with nearest-
// neighbor magnification on the display side (see canvas.rs's sampler
// selection). Real pixel art wants a binary in/out per pixel.
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) local: vec2<f32>,   // -1..1 quad-local coords
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(input.position, 0.0, 1.0);
    out.local = input.local;
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let dist = length(input.local);
    // 1.0 inside the dab radius, 0.0 outside — no intermediate values,
    // unlike canvas_stamp.wgsl's smoothstep. step(edge, x) is 0 below
    // edge and 1 at/above it, so 1.0 - step(1.0, dist) is 1 inside the
    // radius and 0 outside.
    let alpha = 1.0 - step(1.0, dist);
    return vec4<f32>(input.color.rgb, input.color.a * alpha);
}
