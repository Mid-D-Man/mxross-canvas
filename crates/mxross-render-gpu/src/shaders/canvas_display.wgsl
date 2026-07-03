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
// transparent" indicator. Fixed in SCREEN space (physical pixels), not
// canvas UV space — this is the standard behavior in Photoshop/Krita/
// Procreate: the checker pattern stays a constant on-screen cell size
// and doesn't zoom with the canvas content. Doing it in UV space (the
// old approach) meant cells grew huge when zoomed in and shrank into a
// shimmering, aliased mess when zoomed out — this is the "checker
// pattern with the zoom" issue.
//
// `frag_coord` is `@builtin(position)` read in the fragment stage,
// which WGSL/the rasterizer resolves to the actual window-space pixel
// coordinate of this fragment (NOT the clip-space value the vertex
// stage wrote into the same field) — equivalent to GLSL's gl_FragCoord.
// So no extra uniform is needed to know "current zoom/resolution": the
// hardware already hands us physical pixel coordinates directly.
const CHECKER_CELL_PX: f32 = 24.0;

fn checker_color(frag_coord: vec2<f32>) -> vec3<f32> {
    let cell = floor(frag_coord / CHECKER_CELL_PX);
    let parity = (cell.x + cell.y) % 2.0;
    return mix(vec3<f32>(0.82, 0.82, 0.82), vec3<f32>(0.64, 0.64, 0.64), parity);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let canvas_color = textureSample(canvas_texture, canvas_sampler, input.uv);
    // background.a selects checker (0.0) vs solid color (1.0) as the
    // backdrop; the canvas's own alpha then composites over *that*.
    let backdrop = mix(checker_color(input.clip_position.xy), background.rgb, background.a);
    let rgb = mix(backdrop, canvas_color.rgb, canvas_color.a);
    return vec4<f32>(rgb, 1.0);
}
