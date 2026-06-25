// Position is already in canvas-texture NDC, computed on the CPU side
// (canvas.rs::stamp) — this shader's only job is the circular falloff.
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
    // Soft anti-aliased edge instead of a hard circle cutoff.
    let alpha = 1.0 - smoothstep(0.8, 1.0, dist);
    return vec4<f32>(input.color.rgb, input.color.a * alpha);
}
