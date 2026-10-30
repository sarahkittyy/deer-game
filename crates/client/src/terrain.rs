//! The ground and the lakes. The world is cut into chunks; each chunk root is [`Wrapped`], and
//! everything static that stands in a chunk (ground, water, trees, rocks) hangs off it, so the
//! wrap only ever moves the chunk roots.

use crate::wrap::Wrapped;
use crate::{GameWorld, water::WaterMaterial};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use deer_shared::{
    noise::{hash3, unit},
    world::{CELL, WATER_LEVEL, WORLD_SIZE, World},
};

pub const CHUNK: f32 = 32.0;
pub const CHUNKS: usize = (WORLD_SIZE / CHUNK) as usize;
const CELLS: usize = (CHUNK / CELL) as usize;

/// Root of one chunk.
#[derive(Component)]
pub struct Chunk;

/// Chunk root entities by `j * CHUNKS + i`, so other modules can parent things to them.
#[derive(Resource)]
pub struct Chunks(pub Vec<Entity>);

impl Chunks {
    /// The chunk holding wrapped position (x, z), and the position local to that chunk.
    pub fn locate(&self, x: f32, z: f32) -> (Entity, Vec2) {
        let x = x.rem_euclid(WORLD_SIZE);
        let z = z.rem_euclid(WORLD_SIZE);
        let i = ((x / CHUNK) as usize).min(CHUNKS - 1);
        let j = ((z / CHUNK) as usize).min(CHUNKS - 1);
        (
            self.0[j * CHUNKS + i],
            Vec2::new(x - i as f32 * CHUNK, z - j as f32 * CHUNK),
        )
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, spawn_chunks).add_systems(
        Update,
        (reset, spawn_chunks.run_if(not(resource_exists::<Chunks>))).chain(),
    );
}

/// A new world: the old ground (and everything standing on it) goes; `spawn_chunks` runs
/// again once `Chunks` is gone.
fn reset(
    mut commands: Commands,
    mut events: MessageReader<crate::net::NetEvent>,
    chunks: Query<Entity, With<Chunk>>,
) {
    if events
        .read()
        .any(|e| matches!(e, crate::net::NetEvent::NewWorld))
    {
        for e in &chunks {
            commands.entity(e).despawn();
        }
        commands.remove_resource::<Chunks>();
    }
}

fn spawn_chunks(
    mut commands: Commands,
    world: Res<GameWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    water: Res<WaterMaterial>,
) {
    let ground = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.92,
        reflectance: 0.25,
        ..default()
    });
    let water_mesh = meshes.add(
        Plane3d::new(Vec3::Y, Vec2::splat(CHUNK * 0.5))
            .mesh()
            .subdivisions(0),
    );
    let mut roots = Vec::with_capacity(CHUNKS * CHUNKS);
    for j in 0..CHUNKS {
        for i in 0..CHUNKS {
            let origin = Vec3::new(i as f32 * CHUNK, 0.0, j as f32 * CHUNK);
            let (mesh, has_water) = chunk_mesh(&world.0, i, j);
            let root = commands
                .spawn((
                    Chunk,
                    Wrapped(origin),
                    Transform::from_translation(origin),
                    Visibility::default(),
                ))
                .with_child((Mesh3d(meshes.add(mesh)), MeshMaterial3d(ground.clone())))
                .id();
            if has_water {
                commands.entity(root).with_child((
                    Mesh3d(water_mesh.clone()),
                    MeshMaterial3d(water.0.clone()),
                    Transform::from_xyz(CHUNK * 0.5, WATER_LEVEL, CHUNK * 0.5),
                ));
            }
            roots.push(root);
        }
    }
    commands.insert_resource(Chunks(roots));
}

/// Flat-shaded ground for one chunk, coloured per triangle. Also says whether any of it dips
/// under the water, so the chunk gets a water surface.
fn chunk_mesh(world: &World, ci: usize, cj: usize) -> (Mesh, bool) {
    let mut positions = Vec::with_capacity(CELLS * CELLS * 6);
    let mut normals = Vec::with_capacity(CELLS * CELLS * 6);
    let mut colors = Vec::with_capacity(CELLS * CELLS * 6);
    let mut has_water = false;
    let gi0 = (ci * CELLS) as i64;
    let gj0 = (cj * CELLS) as i64;
    for j in 0..CELLS as i64 {
        for i in 0..CELLS as i64 {
            let p = |di: i64, dj: i64| {
                let (gi, gj) = (gi0 + i + di, gj0 + j + dj);
                Vec3::new(
                    (i + di) as f32 * CELL,
                    world.sample(gi, gj),
                    (j + dj) as f32 * CELL,
                )
            };
            let (p00, p10, p01, p11) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
            // Same split as `World::height`: (0,0)-(1,1) diagonal. Counter-clockwise from above.
            for (t, tri) in [[p00, p11, p10], [p00, p01, p11]].into_iter().enumerate() {
                let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
                let center = (tri[0] + tri[1] + tri[2]) / 3.0;
                let wx = ci as f32 * CHUNK + center.x;
                let wz = cj as f32 * CHUNK + center.z;
                if tri.iter().any(|v| v.y < WATER_LEVEL) {
                    has_water = true;
                }
                let jitter = unit(hash3(
                    world.seed,
                    (gi0 + i) as u64 * 2 + t as u64,
                    (gj0 + j) as u64,
                ));
                let c = ground_color(world, wx, wz, center.y, n.y, jitter);
                for v in tri {
                    positions.push(v.to_array());
                    normals.push(n.to_array());
                    colors.push(c);
                }
            }
        }
    }
    let count = positions.len() as u32;
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32((0..count).collect()));
    (mesh, has_water)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Mossy meadow, darker leaf-litter under the trees, mud by the water, stone on steep slopes.
fn ground_color(world: &World, x: f32, z: f32, y: f32, ny: f32, jitter: f32) -> [f32; 4] {
    let meadow = [0.36, 0.47, 0.20];
    let moss = [0.27, 0.36, 0.15];
    let litter = [0.34, 0.29, 0.17];
    let mud = [0.42, 0.36, 0.25];
    let lakebed = [0.26, 0.23, 0.16];
    let stone = [0.42, 0.40, 0.36];

    let forest = world.forest_density(x, z);
    let mut c = mix(meadow, moss, forest * 1.4);
    c = mix(c, litter, (forest - 0.55) * 2.0);
    c = mix(c, mud, 1.0 - (y - WATER_LEVEL) / 0.9);
    c = mix(c, lakebed, -(y - WATER_LEVEL) / 0.6);
    c = mix(c, stone, (0.86 - ny) * 6.0);
    let k = 0.9 + jitter * 0.2;
    let srgb = Color::srgb(c[0] * k, c[1] * k, c[2] * k).to_linear();
    [srgb.red, srgb.green, srgb.blue, 1.0]
}
