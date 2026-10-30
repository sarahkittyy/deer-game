//! Main-pass vertex shader for foliage: Bevy's mesh vertex shader plus wind.
//! (No skinning or morph targets: plants have neither.)

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::globals,
};
import super::wind;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);

@if(VERTEX_NORMALS)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);

    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    let root = world_from_local[3].xyz;
    world = vec4(world.xyz + wind::current(root, vertex.position, globals.time), world.w);
@if(VERTEX_POSITIONS)
    out.world_position = world;
@if(VERTEX_POSITIONS)
    out.position = position_world_to_clip(world.xyz);

@if(VERTEX_UVS_A)
    out.uv = vertex.uv;
@if(VERTEX_UVS_B)
    out.uv_b = vertex.uv_b;
@if(VERTEX_TANGENTS)
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_in.instance_index);
@if(VERTEX_COLORS)
    out.color = vertex.color;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_in.instance_index, world_from_local[3]);

    return out;
}
