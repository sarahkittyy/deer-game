//! Lake surface for the WebGL2 build, which has no deferred lighting or screen-space
//! reflections: the same rippled normals, lit in the forward pass, with the sky's reflection
//! faked by a Fresnel term (strong at grazing angles, faint looking straight down).

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, globals},
};

struct WaterSettings {
    octave_vectors: array<vec4<f32>, 2>,
    octave_scales: vec4<f32>,
    octave_strengths: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var water_normals_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var water_normals_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var<uniform> water_settings: WaterSettings;

/// Luminance of the reflected sky (cd/m²), before the camera's exposure.
const SKY: vec3<f32> = vec3<f32>(2300.0, 2700.0, 3400.0);

fn sample_noise_octave(uv: vec2<f32>, strength: f32) -> vec3<f32> {
    let N = textureSample(water_normals_texture, water_normals_sampler, uv).rbg * 2.0 - 1.0;
    return normalize(mix(vec3(0.0, 1.0, 0.0), N, strength));
}

fn sample_noise(uv: vec2<f32>, time: f32) -> vec3<f32> {
    let uv0 = uv * water_settings.octave_scales[0] + water_settings.octave_vectors[0].xy * time;
    let uv1 = uv * water_settings.octave_scales[1] + water_settings.octave_vectors[0].zw * time;
    let uv2 = uv * water_settings.octave_scales[2] + water_settings.octave_vectors[1].xy * time;
    let uv3 = uv * water_settings.octave_scales[3] + water_settings.octave_vectors[1].zw * time;
    return normalize(
        sample_noise_octave(uv0, water_settings.octave_strengths[0]) +
        sample_noise_octave(uv1, water_settings.octave_strengths[1]) +
        sample_noise_octave(uv2, water_settings.octave_strengths[2]) +
        sample_noise_octave(uv3, water_settings.octave_strengths[3])
    );
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let uv = in.world_position.xz / 512.0;
    let n = sample_noise(uv, globals.time);
    pbr_input.N = n;

    var out: FragmentOutput;
    var color = apply_pbr_lighting(pbr_input);
    let v = normalize(view.world_position.xyz - in.world_position.xyz);
    let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
    color = vec4(color.rgb + SKY * view.exposure * fresnel, color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, color);
    return out;
}
