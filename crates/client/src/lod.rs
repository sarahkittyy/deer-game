//! Cheaper versions of the tree meshes for distant trees, made when the models load.
//!
//! - Solid parts (bark) are simplified by vertex clustering: vertices that fall in the same
//!   grid cell merge into one, and triangles that collapse disappear.
//! - Leaf parts are cards (small alpha-tested quads), which clustering would shred. Instead a
//!   fraction of the cards is kept and each kept card is grown, so the crown keeps its outline
//!   and roughly its coverage with far fewer triangles (and far less overdraw).

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues as V},
    prelude::*,
};
use deer_shared::noise::{hash64, unit};
use std::collections::HashMap;

fn positions(mesh: &Mesh) -> Option<&Vec<[f32; 3]>> {
    match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        V::Float32x3(p) => Some(p),
        _ => None,
    }
}

fn indices(mesh: &Mesh, count: usize) -> Vec<u32> {
    match mesh.indices() {
        Some(i) => i.iter().map(|i| i as u32).collect(),
        None => (0..count as u32).collect(),
    }
}

/// Copies the attributes the materials use (position, normal, uv, colour) for the vertices
/// in `keep`, in that order.
fn gather(mesh: &Mesh, keep: &[u32]) -> Mesh {
    let mut out = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let pick3 = |v: &Vec<[f32; 3]>| keep.iter().map(|&i| v[i as usize]).collect::<Vec<_>>();
    if let Some(V::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        out.insert_attribute(Mesh::ATTRIBUTE_POSITION, pick3(p));
    }
    if let Some(V::Float32x3(n)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, pick3(n));
    }
    if let Some(V::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        let v: Vec<[f32; 2]> = keep.iter().map(|&i| uv[i as usize]).collect();
        out.insert_attribute(Mesh::ATTRIBUTE_UV_0, v);
    }
    if let Some(V::Float32x4(c)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        let v: Vec<[f32; 4]> = keep.iter().map(|&i| c[i as usize]).collect();
        out.insert_attribute(Mesh::ATTRIBUTE_COLOR, v);
    }
    out
}

/// Bark and other solid parts: merges vertices within `cell` (model units) of each other.
pub fn cluster(mesh: &Mesh, cell: f32) -> Option<Mesh> {
    let pos = positions(mesh)?;
    let idx = indices(mesh, pos.len());
    // One representative vertex per occupied cell; the first one seen keeps its uv/normal.
    let mut cells: HashMap<(i32, i32, i32), u32> = HashMap::new();
    let mut remap = vec![0u32; pos.len()];
    let mut reps: Vec<u32> = Vec::new();
    let mut sums: Vec<(Vec3, u32)> = Vec::new();
    for (i, p) in pos.iter().enumerate() {
        let k = (
            (p[0] / cell).floor() as i32,
            (p[1] / cell).floor() as i32,
            (p[2] / cell).floor() as i32,
        );
        let r = *cells.entry(k).or_insert_with(|| {
            reps.push(i as u32);
            sums.push((Vec3::ZERO, 0));
            (reps.len() - 1) as u32
        });
        remap[i] = r;
        sums[r as usize].0 += Vec3::from(*p);
        sums[r as usize].1 += 1;
    }
    let mut tris = Vec::with_capacity(idx.len());
    let mut seen = std::collections::HashSet::new();
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (
            remap[t[0] as usize],
            remap[t[1] as usize],
            remap[t[2] as usize],
        );
        if a == b || b == c || a == c {
            continue;
        }
        // The same triangle can survive twice from different originals.
        let mut key = [a, b, c];
        key.sort();
        if seen.insert(key) {
            tris.extend([a, b, c]);
        }
    }
    if tris.is_empty() {
        return None;
    }
    let mut out = gather(mesh, &reps);
    // Merged vertices sit at the average of what they replaced.
    let avg: Vec<[f32; 3]> = sums
        .iter()
        .map(|(s, n)| (*s / *n as f32).to_array())
        .collect();
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, avg);
    out.insert_indices(Indices::U32(tris));
    if out.attribute(Mesh::ATTRIBUTE_UV_0).is_some()
        && out.attribute(Mesh::ATTRIBUTE_NORMAL).is_some()
    {
        // Bark has a normal map, which needs tangents.
        let _ = out.generate_tangents();
    }
    Some(out)
}

/// Leaf cards: keeps about `keep` of them (whole cards, found as connected triangles) and
/// grows each kept card by `grow` about its middle.
pub fn thin_cards(mesh: &Mesh, keep: f32, grow: f32, seed: u64) -> Option<Mesh> {
    let pos = positions(mesh)?;
    let idx = indices(mesh, pos.len());
    // Union-find over vertices joined by triangles.
    let mut parent: Vec<u32> = (0..pos.len() as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for t in idx.chunks_exact(3) {
        let a = find(&mut parent, t[0]);
        for &v in &t[1..] {
            let b = find(&mut parent, v);
            if a != b {
                parent[b as usize] = a;
            }
        }
    }
    let roots: Vec<u32> = (0..pos.len() as u32)
        .map(|v| find(&mut parent, v))
        .collect();
    // Middle of each card.
    let mut mids: HashMap<u32, (Vec3, u32)> = HashMap::new();
    for (v, &r) in roots.iter().enumerate() {
        let e = mids.entry(r).or_insert((Vec3::ZERO, 0));
        e.0 += Vec3::from(pos[v]);
        e.1 += 1;
    }
    let kept = |r: u32| unit(hash64(seed ^ (r as u64).wrapping_mul(0x9E37))) < keep;
    let mut tris = Vec::new();
    for t in idx.chunks_exact(3) {
        if kept(roots[t[0] as usize]) {
            tris.extend_from_slice(t);
        }
    }
    if tris.is_empty() {
        return None;
    }
    let mut out = gather(mesh, &(0..pos.len() as u32).collect::<Vec<_>>());
    let grown: Vec<[f32; 3]> = pos
        .iter()
        .zip(&roots)
        .map(|(p, r)| {
            let (s, n) = mids[r];
            let m = s / n as f32;
            (m + (Vec3::from(*p) - m) * grow).to_array()
        })
        .collect();
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, grown);
    out.insert_indices(Indices::U32(tris));
    Some(out)
}

/// Triangle count, for logging.
pub fn triangles(mesh: &Mesh) -> usize {
    mesh.indices().map(|i| i.len()).unwrap_or(0) / 3
}
