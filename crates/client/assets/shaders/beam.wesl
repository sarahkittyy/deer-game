//! A shaft of sunlight: bright in the middle, falling off sharply at its edges (seen side-on,
//! from how squarely the surface faces the eye), fading in below the canopy and out just above
//! the ground, with a slow shimmer of dust. Drawn additively.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
};

struct Beam {
    color: vec4<f32>,
    /// x: fade, y: lowest ground under the beam, z: top height, w: per-beam seed.
    params: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> beam: Beam;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position.xyz;
    let to_eye = view.world_position - p;
    let V = normalize(to_eye);
    let N = normalize(in.world_normal);
    // Side-on, the beam's silhouette: crisp edges rather than a soft glow.
    let facing = abs(dot(N, V));
    let core = smoothstep(0.3, 0.4, facing);

    let ground = beam.params.y;
    let top = beam.params.z;
    let h = clamp((p.y - ground) / (top - ground), 0.0, 1.0);
    // Strongest just under the leaves, thinning towards the ground but reaching it (the end
    // is buried; `ground` is the lowest ground under the beam).
    let along = smoothstep(0.0, 0.4 / (top - ground), h) * (1.0 - smoothstep(0.75, 1.0, h)) * mix(0.55, 1.0, h);

    let seed = beam.params.w;
    // A bundle of streaks rather than one flat sheet: bright rays around the beam's
    // circumference, uneven so they do not look ruled.
    let u = in.uv.x * 6.2831853;
    let rays = pow(0.5 + 0.5 * sin(u * 5.0 + seed), 3.0) * 0.7
        + pow(0.5 + 0.5 * sin(u * 9.0 - seed * 1.7), 6.0) * 0.5
        + 0.12;

    // Dust drifting through the light.
    let t = globals.time;
    let dust = 0.8 + 0.2 * sin(t * 0.6 + p.y * 1.7 + seed) * sin(t * 0.37 + p.x * 2.3 - seed);

    // Do not blind the camera when it stands inside a beam.
    let near = smoothstep(1.5, 6.0, length(to_eye));

    let a = core * rays * along * dust * near * beam.params.x * 0.9;
    return vec4(beam.color.rgb * a, a);
}
