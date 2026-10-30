//! Wind for foliage, shared by the main pass and the prepass. Displacement grows with height
//! in the model (roots stay put), a slow sway across the whole plant plus a quick flutter.

struct Wind {
    /// x: sway amplitude (m), y: flutter amplitude (m), z: 1 / model height, w: unused.
    params: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> wind: Wind;

fn offset(world: vec3<f32>, local: vec3<f32>, t: f32) -> vec3<f32> {
    let h = clamp(local.y * wind.params.z, 0.0, 1.5);
    let bend = h * h;
    // Neighbouring plants move together: the phase drifts slowly across the ground.
    let phase = dot(world.xz, vec2(0.071, 0.053));
    let gust = 0.65 + 0.35 * sin(t * 0.23 + phase * 0.4);
    let sway = (sin(t * 0.85 + phase) * 0.7 + sin(t * 1.9 + phase * 1.7) * 0.3) * gust;
    let leaf = dot(world, vec3(1.9, 1.3, 2.3));
    let flutter = vec3(sin(t * 7.1 + leaf), sin(t * 5.3 + leaf * 1.3) * 0.5, cos(t * 6.4 + leaf * 0.7));
    return vec3(0.8, 0.0, 0.6) * sway * bend * wind.params.x
        + flutter * h * wind.params.y;
}

// The clock comes from Bevy's globals (the caller passes them in: the main pass and the
// prepass bind them at different slots), so the materials never change and are never
// re-uploaded or re-specialised.
fn current(world: vec3<f32>, local: vec3<f32>, time: f32) -> vec3<f32> {
    return offset(world, local, time);
}

fn previous(world: vec3<f32>, local: vec3<f32>, time: f32, delta: f32) -> vec3<f32> {
    return offset(world, local, time - delta);
}
