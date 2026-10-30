//! Lake surface: four scrolling octaves of a normal map ripple the surface, and the deferred
//! lighting pass (with screen-space reflections) does the rest. Adapted from Bevy's water example.

import bevy_pbr::{
    deferred::functions::deferred_output,
    render::pbr_fragment::pbr_input_from_standard_material,
    prepass::io::{VertexOutput, FragmentOutput},
};
import bevy_render::globals::Globals;

struct WaterSettings {
    octave_vectors: array<vec4<f32>, 2>,
    octave_scales: vec4<f32>,
    octave_strengths: vec4<f32>,
}

@group(0) @binding(1) var<uniform> globals: Globals;

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var water_normals_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var water_normals_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var<uniform> water_settings: WaterSettings;

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
    // World-space uv, so neighbouring chunks' water lines up (and wraps: 512 is a whole number
    // of every octave's tiles).
    let uv = in.world_position.xz / 512.0;
    pbr_input.N = sample_noise(uv, globals.time);
    return deferred_output(in, pbr_input);
}
