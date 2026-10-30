//! Shafts of sunlight through gaps in the canopy, drawn as sharp-edged beams rather than fog.
//!
//! Around the player, the ground is sampled on a grid; a spot is sunlit when a ray from it
//! towards the sun misses every tree crown (each crown is an ellipsoid). Neighbouring sunlit
//! spots form a gap, and each gap gets one beam leaning along the sun, so the beams land on the
//! same pools of light the shadows leave on the ground.

use crate::{
    GameWorld,
    env::sun_direction,
    vegetation::{TreeShape, TreeShapes},
    wrap::{Focus, Wrapped},
};
use bevy::{
    light::NotShadowCaster,
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use deer_shared::world::{World, wrap, wrap_delta};
use std::collections::{HashMap, HashSet};

/// Ground within this distance of the player gets beams.
const RANGE: f32 = 36.0;
const CELL: f32 = 1.25;
/// Rebuild the beams after the player has moved this far.
const REBUILD: f32 = 5.0;
/// Crowns are leafy, not solid: shrink them a little so the odd gap shows through.
const CROWN_FILL: f32 = 0.92;
/// Gaps bigger than this many cells are clearings, open to the sky, not shafts.
const MAX_GAP_CELLS: usize = 90;
/// Height above the ground where the canopy, and so each beam, begins.
const CANOPY_TOP: f32 = 24.0;
const FADE_SECS: f32 = 1.2;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BeamMaterial {
    #[uniform(0)]
    beam: BeamUniform,
}

#[derive(ShaderType, Debug, Clone, Copy)]
struct BeamUniform {
    color: Vec4,
    /// x: fade (0..1), y: lowest ground under the beam, z: top height, w: seed for the shimmer.
    params: Vec4,
}

impl Material for BeamMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/beam.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
}

/// Crowns by 16 m bucket, for the occlusion rays.
#[derive(Resource)]
struct CrownIndex {
    buckets: HashMap<(i32, i32), Vec<u32>>,
}

const BUCKET: f32 = 16.0;

#[derive(Component)]
struct Beam {
    key: (i32, i32),
    fade: f32,
    leaving: bool,
}

#[derive(Resource, Default)]
struct BeamState {
    built_at: Option<Vec3>,
    mesh: Option<Handle<Mesh>>,
}

pub fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<BeamMaterial>::default())
        .init_resource::<BeamState>()
        .add_systems(
            Update,
            (
                reset,
                index_crowns.run_if(resource_added::<TreeShapes>),
                rebuild
                    .run_if(resource_exists::<CrownIndex>)
                    .run_if(|| !crate::debug::off("beams")),
                fade,
            )
                .chain(),
        );
}

/// A new world: the old beams go, and are worked out again once the new trees stand.
fn reset(
    mut commands: Commands,
    mut events: MessageReader<crate::net::NetEvent>,
    beams: Query<Entity, With<Beam>>,
    mut state: ResMut<BeamState>,
) {
    if events
        .read()
        .any(|e| matches!(e, crate::net::NetEvent::NewWorld))
    {
        for e in &beams {
            commands.entity(e).despawn();
        }
        state.built_at = None;
        commands.remove_resource::<CrownIndex>();
    }
}

fn index_crowns(mut commands: Commands, shapes: Res<TreeShapes>) {
    let mut buckets: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, s) in shapes.0.iter().enumerate() {
        let Some(s) = s else { continue };
        if s.crown_radii == Vec3::ZERO {
            continue;
        }
        let k = (
            (wrap(s.crown_center.x) / BUCKET) as i32,
            (wrap(s.crown_center.z) / BUCKET) as i32,
        );
        buckets.entry(k).or_default().push(i as u32);
    }
    commands.insert_resource(CrownIndex { buckets });
}

/// Whether the crown blocks the ray from `o` along `d` (wrapped coordinates).
fn hits(c: &TreeShape, o: Vec3, d: Vec3) -> bool {
    let rel = Vec3::new(
        wrap_delta(o.x - c.crown_center.x),
        o.y - c.crown_center.y,
        wrap_delta(o.z - c.crown_center.z),
    );
    let r = c.crown_radii * CROWN_FILL;
    let q = rel / r;
    let v = d / r;
    let a = v.dot(v);
    let b = 2.0 * q.dot(v);
    let cc = q.dot(q) - 1.0;
    let disc = b * b - 4.0 * a * cc;
    if disc < 0.0 {
        return false;
    }
    // The far intersection in front of the origin is enough: the ray passes through.
    (-b + disc.sqrt()) / (2.0 * a) > 0.0
}

fn sunlit(
    index: &CrownIndex,
    shapes: &TreeShapes,
    o: Vec3,
    d: Vec3,
    seen: &mut HashSet<u32>,
) -> bool {
    seen.clear();
    // Walk the ray's shadow on the ground until it is above every crown.
    let horiz = Vec2::new(d.x, d.z).length().max(1e-3);
    let reach = (CANOPY_TOP + 16.0) / d.y.max(0.05) * horiz;
    let steps = (reach / (BUCKET * 0.5)).ceil() as i32;
    for s in 0..=steps {
        let t = s as f32 * BUCKET * 0.5 / horiz;
        let p = o + d * t;
        let bx = (wrap(p.x) / BUCKET) as i32;
        let bz = (wrap(p.z) / BUCKET) as i32;
        let n = (deer_shared::world::WORLD_SIZE / BUCKET) as i32;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let k = ((bx + dx).rem_euclid(n), (bz + dz).rem_euclid(n));
                let Some(list) = index.buckets.get(&k) else {
                    continue;
                };
                for &i in list {
                    if !seen.insert(i) {
                        continue;
                    }
                    if let Some(c) = &shapes.0[i as usize]
                        && hits(c, o, d)
                    {
                        return false;
                    }
                }
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    focus: Res<Focus>,
    world: Res<GameWorld>,
    shapes: Res<TreeShapes>,
    index: Res<CrownIndex>,
    mut state: ResMut<BeamState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BeamMaterial>>,
    mut beams: Query<&mut Beam>,
) {
    let f = focus.0;
    if state
        .built_at
        .is_some_and(|b| Vec2::new(b.x - f.x, b.z - f.z).length() < REBUILD)
    {
        return;
    }
    state.built_at = Some(f);
    let started = bevy::platform::time::Instant::now();
    let mesh = state
        .mesh
        .get_or_insert_with(|| meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(16)))
        .clone();
    let w: &World = &world.0;
    let d = sun_direction();

    // Sunlit cells around the player, keyed by absolute (wrapped) cell coordinates.
    let n = (RANGE / CELL) as i32;
    let ci = (f.x / CELL).floor() as i32;
    let cj = (f.z / CELL).floor() as i32;
    let mut lit = HashSet::new();
    let mut seen = HashSet::new();
    for j in -n..=n {
        for i in -n..=n {
            if (i * i + j * j) as f32 > (n * n) as f32 {
                continue;
            }
            let (x, z) = (
                ((ci + i) as f32 + 0.5) * CELL,
                ((cj + j) as f32 + 0.5) * CELL,
            );
            if w.is_water(x, z) {
                continue;
            }
            let o = Vec3::new(wrap(x), w.height(x, z) + 0.3, wrap(z));
            if sunlit(&index, &shapes, o, d, &mut seen) {
                lit.insert((ci + i, cj + j));
            }
        }
    }

    // Group into gaps (4-connected), one beam each.
    let mut wanted: HashMap<(i32, i32), (Vec2, usize)> = HashMap::new();
    let mut done = HashSet::new();
    for &start in &lit {
        if done.contains(&start) {
            continue;
        }
        let mut stack = vec![start];
        let mut cells = Vec::new();
        done.insert(start);
        while let Some(c) = stack.pop() {
            cells.push(c);
            for nb in [
                (c.0 + 1, c.1),
                (c.0 - 1, c.1),
                (c.0, c.1 + 1),
                (c.0, c.1 - 1),
            ] {
                if lit.contains(&nb) && done.insert(nb) {
                    stack.push(nb);
                }
            }
        }
        if cells.len() > MAX_GAP_CELLS {
            continue;
        }
        let key = *cells.iter().min().unwrap();
        let sum = cells.iter().fold(Vec2::ZERO, |a, c| {
            a + Vec2::new(c.0 as f32 + 0.5, c.1 as f32 + 0.5)
        });
        wanted.insert(key, (sum / cells.len() as f32 * CELL, cells.len()));
    }

    debug!(
        "light shafts: {} sunlit cells, {} gaps, {:.1} ms",
        lit.len(),
        wanted.len(),
        started.elapsed().as_secs_f32() * 1000.0
    );
    // Keep beams that are still wanted, fade out the rest, add the new ones.
    let mut have = HashSet::new();
    for mut b in &mut beams {
        if wanted.contains_key(&b.key) && !b.leaving {
            have.insert(b.key);
        } else {
            b.leaving = true;
        }
    }
    for (key, (c, count)) in wanted {
        if have.contains(&key) {
            continue;
        }
        let ground = w.height(c.x, c.y);
        let area = count as f32 * CELL * CELL;
        // A slanted beam's footprint is stretched by 1 / sin(elevation); size the beam so its
        // footprint matches the patch of light.
        let radius = (area * d.y / std::f32::consts::PI).sqrt().clamp(0.3, 2.4);
        // The beam ends below the lowest ground anywhere under its footprint (and its slanted
        // end below that again), so the terrain always hides the end: on a slope a beam cut
        // off at the middle's height would stop in mid-air on the downhill side.
        let reach = radius / d.y + 0.5;
        let low = (0..8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                w.height(c.x + a.cos() * reach, c.y + a.sin() * reach)
            })
            .fold(ground, f32::min);
        let bottom = low - radius * (1.0 - d.y * d.y).sqrt() - 0.3;
        let top = ground + CANOPY_TOP;
        let length = (top - bottom) / d.y;
        let axis = Vec3::new(wrap(c.x), ground, wrap(c.y));
        let foot = axis + d * ((bottom - ground) / d.y);
        let mid = foot + d * length * 0.5;
        let seed = (key.0.wrapping_mul(73856093) ^ key.1.wrapping_mul(19349663)) as u32 % 1000;
        commands.spawn((
            Beam {
                key,
                fade: 0.0,
                leaving: false,
            },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(BeamMaterial {
                beam: BeamUniform {
                    color: Vec4::new(1.0, 0.8, 0.52, 1.0),
                    params: Vec4::new(0.0, low, top, seed as f32),
                },
            })),
            Wrapped(mid),
            Transform::from_translation(mid)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, d))
                .with_scale(Vec3::new(radius, length, radius)),
            NotShadowCaster,
        ));
    }
}

/// Fades beams in and out as they come and go, and away when the camera is about to walk
/// through one (from the inside a beam is just a flat sheet across the view).
fn fade(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<&GlobalTransform, With<crate::env::MainCamera>>,
    mut beams: Query<(Entity, &mut Beam, &Transform, &MeshMaterial3d<BeamMaterial>)>,
    mut materials: ResMut<Assets<BeamMaterial>>,
) {
    let step = time.delta_secs() / FADE_SECS;
    let eye = camera.translation();
    for (e, mut b, t, m) in &mut beams {
        let axis = t.rotation * Vec3::Y;
        let half = t.scale.y * 0.5;
        let along = (eye - t.translation).dot(axis).clamp(-half, half);
        let to_axis = eye.distance(t.translation + axis * along) - t.scale.x;
        let near = ((to_axis - 1.0) / 5.0).clamp(0.0, 1.0);
        b.fade = if b.leaving {
            (b.fade - step).max(0.0)
        } else {
            (b.fade + step).min(1.0)
        };
        if b.leaving && b.fade <= 0.0 {
            materials.remove(&m.0);
            commands.entity(e).despawn();
            continue;
        }
        let shown = b.fade * near * near * (3.0 - 2.0 * near);
        if let Some(mut mat) = materials.get_mut(&m.0)
            && (mat.beam.params.x - shown).abs() > 0.003
        {
            mat.beam.params.x = shown;
        }
    }
}
