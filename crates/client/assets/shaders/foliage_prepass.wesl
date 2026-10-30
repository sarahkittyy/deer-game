//! Prepass (depth, shadows, deferred G-buffer) vertex shader for foliage, matching
//! `foliage.wesl` so shadows and the lit surface sway together.

import bevy_pbr::{
    render::{mesh_functions, view_transformations::position_world_to_clip},
    prepass::io::{Vertex, VertexOutput, decompress_vertex},
};
import super::wind;
import bevy_render::globals::Globals;

// The prepass's view bind group has the globals at binding 1.
@group(0) @binding(1) var<uniform> globals: Globals;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let root = world_from_local[3].xyz;

    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    world = vec4(world.xyz + wind::current(root, vertex.position, globals.time), world.w);
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}

@if(VERTEX_UVS_A)
    out.uv = vertex.uv;
@if(VERTEX_UVS_B)
    out.uv_b = vertex.uv_b;

@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
@if(VERTEX_NORMALS)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);
@if(VERTEX_TANGENTS)
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_in.instance_index);
}

@if(VERTEX_COLORS)
    out.color = vertex.color;

@if(MOTION_VECTOR_PREPASS) {
    let prev_model = mesh_functions::get_previous_world_from_local(vertex_in.instance_index);
    let prev_root = prev_model[3].xyz;
    let prev = mesh_functions::mesh_position_local_to_world(prev_model, vec4<f32>(vertex.position, 1.0));
    out.previous_world_position = vec4(prev.xyz + wind::previous(prev_root, vertex.position, globals.time, globals.delta_time), prev.w);
}

@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_in.instance_index, world_from_local[3]);

    return out;
}
