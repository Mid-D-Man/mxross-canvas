struct Camera {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;

@group(0) @binding(1)
var canvas_texture: texture_2d<f32>;
@group(0) @binding(2)
var canvas_sampler: sampler;

// .rgb = solid background color, .a = mode (0.0 = transparent/checker,
// 1.0 = solid) — see BackgroundUniform on the Rust side.
@group(0) @binding(3)
var<uniform> background: vec4<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(input.position, 1.0);
    out.uv = input.uv;
    return out;
}

// Standard two-shade checkerboard, the universal "this is actually
// transparent" indicator. Scaled in UV space, so checker cells scale
// with zoom rather than staying a fixed screen size — a reasonable
// simplification for now, not something every painting app even agrees
// on (some keep cells screen-space-constant, which would need the
// current zoom/resolution passed in too).
fn checker_color(uv: vec2<f32>) -> vec3<f32> {
    let cell = floor(uv * 32.0);
    let parity = (cell.x + cell.y) % 2.0;
    return mix(vec3<f32>(0.82, 0.82, 0.82), vec3<f32>(0.64, 0.64, 0.64), parity);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let canvas_color = textureSample(canvas_texture, canvas_sampler, input.uv);
    // background.a selects checker (0.0) vs solid color (1.0) as the
    // backdrop; the canvas's own alpha then composites over *that*.
    let backdrop = mix(checker_color(input.uv), background.rgb, background.a);
    let rgb = mix(backdrop, canvas_color.rgb, canvas_color.a);
    return vec4<f32>(rgb, 1.0);
}
