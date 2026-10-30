//! Trees, boulders and ground cover. Models are loaded once as "prefabs" (their meshes and
//! materials, leaves swapped for swaying foliage) and placed as plain mesh entities under the
//! chunk roots. Trees and boulders cover the whole world; grass, ferns, flowers and the like only
//! exist in the chunks near the player.

use crate::{
    GameWorld,
    foliage::{self, Foliage, Sway},
    terrain::{CHUNK, CHUNKS, Chunks},
    wrap::Focus,
};
use bevy::{
    camera::primitives::MeshAabb,
    camera::visibility::VisibilityRange,
    gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode},
    light::NotShadowCaster,
    pbr::gltf::standard_material_from_gltf_material,
    prelude::*,
};
use deer_shared::{
    noise::{hash3, hash64, unit},
    world::{WATER_LEVEL, World, wrap, wrap_delta},
};
use std::collections::HashMap;

/// Ground cover exists in chunks whose centre is within this distance of the player: its
/// furthest visible distance plus most of a chunk, so what is drawn is nearly always there.
fn detail_radius() -> f32 {
    if crate::env::lite() { 64.0 } else { 80.0 }
}
const DETAIL_CELL: f32 = 1.6;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Model {
    Pine(u8),
    Common(u8),
    Twisted(u8),
    /// The twisted trees' big crowns with green leaves: the canopy's broadleaf trees.
    TwistedGreen(u8),
    /// Wide spreading maples (Ultimate Stylized Nature, in MegaKit textures): roof trees.
    Maple(u8),
    Dead(u8),
    Stump,
    Rock(u8),
    Bush,
    FlowerBush,
    GrassShort,
    GrassTall,
    GrassWispy,
    Fern,
    Flower3,
    Flower4,
    Clover,
    Plant7,
    Mushroom,
    Laetiporus,
    Pebble(u8),
    CatTail(u8),
    Lily(u8),
    LilyFlower,
    Calla,
    Berry,
    Bird(u8),
}

impl Model {
    fn path(self) -> String {
        let n = "models/nature";
        match self {
            Model::Pine(i) => format!("{n}/Pine_{i}.gltf"),
            Model::Common(i) => format!("{n}/CommonTree_{i}.gltf"),
            Model::Twisted(i) => format!("{n}/TwistedTree_{i}.gltf"),
            Model::TwistedGreen(i) => format!("{n}/TwistedTree_{i}_Green.gltf"),
            Model::Maple(i) => format!("{n}/maple/MapleTree_{i}_MK.gltf"),
            Model::Dead(i) => format!("{n}/DeadTree_{i}.gltf"),
            Model::Stump => format!("{n}/TreeStump_Moss.glb"),
            Model::Rock(i) => format!("{n}/Rock_Medium_{i}.gltf"),
            Model::Bush => format!("{n}/Bush_Common.gltf"),
            Model::FlowerBush => format!("{n}/Bush_Common_Flowers.gltf"),
            Model::GrassShort => format!("{n}/Grass_Common_Short.gltf"),
            Model::GrassTall => format!("{n}/Grass_Common_Tall.gltf"),
            Model::GrassWispy => format!("{n}/Grass_Wispy_Short.gltf"),
            Model::Fern => format!("{n}/Fern_1.gltf"),
            Model::Flower3 => format!("{n}/Flower_3_Group.gltf"),
            Model::Flower4 => format!("{n}/Flower_4_Group.gltf"),
            Model::Clover => format!("{n}/Clover_1.gltf"),
            Model::Plant7 => format!("{n}/Plant_7.gltf"),
            Model::Mushroom => format!("{n}/Mushroom_Common.gltf"),
            Model::Laetiporus => format!("{n}/Mushroom_Laetiporus.gltf"),
            Model::Pebble(i) => format!("{n}/Pebble_Round_{i}.gltf"),
            Model::CatTail(i) => format!("models/pond/CatTail_{i}.gltf"),
            Model::Lily(i) => format!("models/pond/Water_Lily_Leaf_{i}.gltf"),
            Model::LilyFlower => "models/pond/Water_Lily_Blossom_1.gltf".into(),
            Model::Calla => "models/pond/Swamp_Calla_1.gltf".into(),
            Model::Berry => format!("{n}/Berry.glb"),
            Model::Bird(i) => format!("models/pond/Bird_{i}.glb"),
        }
    }

    fn sway(self) -> Option<Sway> {
        match self {
            Model::Pine(_)
            | Model::Common(_)
            | Model::Twisted(_)
            | Model::TwistedGreen(_)
            | Model::Maple(_) => Some(Sway::TREE),
            Model::Bush | Model::FlowerBush => Some(Sway::BUSH),
            Model::GrassShort | Model::GrassTall | Model::GrassWispy | Model::Fern => {
                Some(Sway::GRASS)
            }
            Model::Flower3 | Model::Flower4 | Model::Clover | Model::Plant7 => Some(Sway {
                height: 2.0,
                ..Sway::GRASS
            }),
            Model::CatTail(_) | Model::Calla => Some(Sway::REED),
            _ => None,
        }
    }

    fn all() -> Vec<Model> {
        let mut v = Vec::new();
        for i in 1..=5 {
            v.push(Model::Pine(i));
            v.push(Model::Common(i));
        }
        for i in [1, 3] {
            v.push(Model::Twisted(i));
            v.push(Model::TwistedGreen(i));
        }
        for i in [3, 5] {
            v.push(Model::Maple(i));
        }
        for i in 1..=3 {
            v.push(Model::Dead(i));
            v.push(Model::Rock(i));
            v.push(Model::Pebble(i));
            v.push(Model::CatTail(i));
            v.push(Model::Bird(i));
        }
        v.extend([
            Model::Stump,
            Model::Bush,
            Model::FlowerBush,
            Model::GrassShort,
            Model::GrassTall,
            Model::GrassWispy,
            Model::Fern,
            Model::Flower3,
            Model::Flower4,
            Model::Clover,
            Model::Plant7,
            Model::Mushroom,
            Model::Laetiporus,
            Model::Lily(1),
            Model::Lily(2),
            Model::LilyFlower,
            Model::Calla,
            Model::Berry,
        ]);
        v
    }
}

#[derive(Clone)]
pub enum Mat {
    Standard(Handle<StandardMaterial>),
    Foliage(Handle<Foliage>),
}

#[derive(Clone)]
pub struct Part {
    pub mesh: Handle<Mesh>,
    pub material: Mat,
    pub transform: Transform,
    /// Cheaper meshes for distance, nearest first (trees only; `lods[0]` is `mesh`).
    pub lods: Vec<Handle<Mesh>>,
}

/// A tree part's meshes by level of detail; `TreeLods` swaps between them.
#[derive(Component)]
struct LodMeshes(Vec<Handle<Mesh>>);

/// Tree parts by chunk, and the level each chunk is drawn at.
#[derive(Resource)]
struct TreeLods {
    parts: Vec<Vec<Entity>>,
    level: Vec<u8>,
}

/// Chunk-centre distances where trees switch to their next cheaper mesh.
const LOD_DISTANCES: [f32; 2] = [52.0, 105.0];

/// A loaded model, ready to place.
#[derive(Clone)]
pub struct Prefab {
    pub parts: Vec<Part>,
    /// Bounds of the model in its own space.
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    /// Points on visible leaves, with outward normals, in the model's space (bushes only:
    /// where berries can hang).
    pub surface: Vec<(Vec3, Vec3)>,
    /// The trunk traced up from the ground as a stack of circles, in model units: height,
    /// centre (leaning trunks drift off the origin), radius.
    pub trunk: Vec<(f32, Vec2, f32)>,
    /// Bounds of the foliage, if it has any.
    pub crown: Option<(Vec3, Vec3)>,
}

/// One slice of a trunk, for collisions: a circle at some height above the tree's base.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ring {
    /// Height above the ground at the tree, in metres.
    pub y: f32,
    /// Centre, in wrapped world coordinates.
    pub pos: [f32; 2],
    pub r: f32,
}

const MAX_RINGS: usize = 32;
/// Rings are kept up to this height: a deer knocked flying (`knock`) can hit a trunk high up.
const RING_TOP: f32 = 40.0;

/// Where one tree's trunk and crown are, as drawn: for collisions and the light shafts.
#[derive(Clone, Copy, Debug)]
pub struct TreeShape {
    rings: [Ring; MAX_RINGS],
    ring_count: u8,
    /// Ellipsoid around the crown, in wrapped world coordinates.
    pub crown_center: Vec3,
    pub crown_radii: Vec3,
}

impl TreeShape {
    /// The trunk from the ground up, as circles; a leaning trunk's drift sideways with height.
    pub fn rings(&self) -> &[Ring] {
        &self.rings[..self.ring_count as usize]
    }
}

/// Shape of every tree in `World::trees`, by index.
#[derive(Resource)]
pub struct TreeShapes(pub Vec<Option<TreeShape>>);

#[derive(Resource, Default)]
pub struct Prefabs {
    loading: Vec<(Model, Handle<Gltf>)>,
    pub ready: HashMap<Model, Prefab>,
}

impl Prefabs {
    pub fn all_loaded(&self) -> bool {
        self.loading.is_empty() && !self.ready.is_empty()
    }

    /// Spawns `model` as children of `parent`, placed by `t`.
    pub fn spawn(
        &self,
        commands: &mut Commands,
        model: Model,
        parent: Entity,
        t: Transform,
        shadows: bool,
    ) -> Entity {
        self.spawn_parts(commands, model, parent, t, shadows).0
    }

    /// As `spawn`, also returning the entities of the model's parts.
    pub fn spawn_parts(
        &self,
        commands: &mut Commands,
        model: Model,
        parent: Entity,
        t: Transform,
        shadows: bool,
    ) -> (Entity, Vec<Entity>) {
        let mut ids = Vec::new();
        let prefab = &self.ready[&model];
        let root = commands
            .spawn((t, Visibility::default(), ChildOf(parent)))
            .id();
        for part in &prefab.parts {
            let mut e = commands.spawn((Mesh3d(part.mesh.clone()), part.transform, ChildOf(root)));
            // Small things stop mattering long before the fog; big ones are culled by the far
            // plane, which costs nothing per entity.
            if !shadows {
                // WebGL2 cannot dither the fade-out (its shader binding fails to validate), so
                // there they just stop at one distance.
                let end = if crate::env::lite() {
                    45.0..45.0
                } else {
                    55.0..70.0
                };
                e.insert(VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: end,
                    use_aabb: false,
                });
            }
            match &part.material {
                Mat::Standard(h) => e.insert(MeshMaterial3d(h.clone())),
                Mat::Foliage(h) => e.insert(MeshMaterial3d(h.clone())),
            };
            if part.lods.len() > 1 {
                e.insert(LodMeshes(part.lods.clone()));
            }
            ids.push(e.id());
            if !shadows {
                e.insert(NotShadowCaster);
            }
        }
        (root, ids)
    }
}

/// Root of a chunk's ground cover, despawned when the player wanders away.
#[derive(Component)]
struct Detail;

#[derive(Resource, Default)]
struct DetailChunks(HashMap<usize, Entity>);

/// Set once trees and boulders have been placed.
#[derive(Resource)]
pub struct VegetationReady;

pub fn plugin(app: &mut App) {
    app.init_resource::<Prefabs>()
        .init_resource::<DetailChunks>()
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (
                reset,
                build_prefabs,
                place_static.run_if(not(resource_exists::<VegetationReady>)),
                update_detail.run_if(resource_exists::<VegetationReady>),
                update_lods.run_if(resource_exists::<TreeLods>),
            )
                .chain(),
        );
}

/// A new world: trees, rocks and ground cover went with the chunks; forget them so they are
/// placed again for the new world.
fn reset(
    mut commands: Commands,
    mut events: MessageReader<crate::net::NetEvent>,
    mut detail: ResMut<DetailChunks>,
) {
    if events
        .read()
        .any(|e| matches!(e, crate::net::NetEvent::NewWorld))
    {
        commands.remove_resource::<VegetationReady>();
        commands.remove_resource::<TreeShapes>();
        commands.remove_resource::<TreeLods>();
        detail.0.clear();
    }
}

fn load(assets: Res<AssetServer>, mut prefabs: ResMut<Prefabs>) {
    prefabs.loading = Model::all()
        .into_iter()
        .map(|m| (m, assets.load(m.path())))
        .collect();
}

#[allow(clippy::too_many_arguments)]
fn build_prefabs(
    mut prefabs: ResMut<Prefabs>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    gltf_materials: Res<Assets<GltfMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    assets: Res<AssetServer>,
    mut std_materials: ResMut<Assets<StandardMaterial>>,
    mut foliage: ResMut<Assets<Foliage>>,
    mut cache: Local<HashMap<(AssetId<GltfMaterial>, bool), Mat>>,
) {
    if prefabs.loading.is_empty() {
        return;
    }
    let mut still = Vec::new();
    for (model, handle) in std::mem::take(&mut prefabs.loading) {
        // Wait until the file and everything in it (meshes, textures) has loaded.
        if !assets.is_loaded_with_dependencies(&handle) {
            if assets.load_state(&handle).is_failed() {
                error!("failed to load {}", model.path());
            } else {
                still.push((model, handle));
            }
            continue;
        }
        let gltf = gltfs.get(&handle).unwrap();
        let names: HashMap<AssetId<GltfMaterial>, &str> = gltf
            .named_materials
            .iter()
            .map(|(n, h)| (h.id(), n.as_ref()))
            .collect();
        let mut parts = Vec::new();
        let mut surface = Vec::new();
        let mut trunk: Vec<(f32, Vec2, f32)> = Vec::new();
        let mut crown: Option<(Vec3, Vec3)> = None;
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for node in &gltf.nodes {
            let Some(node) = gltf_nodes.get(node) else {
                continue;
            };
            let Some(mesh) = node.mesh.as_ref().and_then(|m| gltf_meshes.get(m)) else {
                continue;
            };
            for prim in &mesh.primitives {
                if let Some(aabb) = meshes.get(&prim.mesh).and_then(|m| m.get_aabb()) {
                    let (a, b) = (Vec3::from(aabb.min()), Vec3::from(aabb.max()));
                    lo = lo.min(node.transform.transform_point(a));
                    hi = hi.max(node.transform.transform_point(b));
                }
                let Some(mat) = &prim.material else { continue };
                let name = names.get(&mat.id()).copied().unwrap_or("");
                let leafy = ["Leaves", "Leaf", "Grass", "Flowers", "Mat"]
                    .iter()
                    .any(|k| name.contains(k))
                    && model.sway().is_some();
                if let Some(mesh) = meshes.get(&prim.mesh) {
                    if leafy {
                        if let Some(aabb) = mesh.get_aabb() {
                            let (a, b) = (
                                node.transform.transform_point(Vec3::from(aabb.min())),
                                node.transform.transform_point(Vec3::from(aabb.max())),
                            );
                            crown = Some(match crown {
                                Some((l, h)) => (l.min(a), h.max(b)),
                                None => (a, b),
                            });
                        }
                    } else if name.contains("Bark") {
                        let traced = trace_trunk(mesh, node.transform);
                        // A model may have several bark parts: keep the trunk (the widest base).
                        if traced.first().map(|r| r.2) > trunk.first().map(|r| r.2) {
                            trunk = traced;
                        }
                        debug!("{model:?} trunk {:.2?}", trunk);
                    }
                }
                if model == Model::Bush
                    && leafy
                    && let Some(mesh) = meshes.get(&prim.mesh)
                {
                    let texture = gltf_materials
                        .get(mat)
                        .and_then(|m| m.base_color_texture.as_ref())
                        .and_then(|t| images.get(t));
                    surface.extend(leaf_surface(mesh, texture, 600).into_iter().map(|(p, n)| {
                        (
                            node.transform.transform_point(p),
                            node.transform.rotation * n,
                        )
                    }));
                }
                let material = cache
                    .entry((mat.id(), leafy))
                    .or_insert_with(|| {
                        let mut base =
                            standard_material_from_gltf_material(gltf_materials.get(mat).unwrap());
                        if name.contains("Bark") {
                            // Bark has no holes; opaque is cheaper than alpha-tested.
                            base.alpha_mode = AlphaMode::Opaque;
                        }
                        base.perceptual_roughness = base.perceptual_roughness.max(0.8);
                        base.reflectance = 0.3;
                        match (leafy, model.sway()) {
                            (true, Some(sway)) => {
                                Mat::Foliage(foliage.add(foliage::make(base, sway)))
                            }
                            _ => Mat::Standard(std_materials.add(base)),
                        }
                    })
                    .clone();
                let mut lods = vec![prim.mesh.clone()];
                if model.sway() == Some(Sway::TREE) || matches!(model, Model::Dead(_)) {
                    let src = meshes.get(&prim.mesh).cloned();
                    if let Some(src) = src {
                        let h = src
                            .get_aabb()
                            .map(|a| a.half_extents.y * 2.0)
                            .unwrap_or(8.0);
                        let made: Vec<Option<Mesh>> = if leafy {
                            vec![
                                crate::lod::thin_cards(&src, 0.42, 1.4, 1),
                                crate::lod::thin_cards(&src, 0.16, 2.1, 2),
                            ]
                        } else {
                            vec![
                                crate::lod::cluster(&src, h * 0.025),
                                crate::lod::cluster(&src, h * 0.06),
                            ]
                        };
                        let before = crate::lod::triangles(&src);
                        for m in made.into_iter().flatten() {
                            debug!(
                                "{:?} {}: lod {} -> {} triangles",
                                model,
                                name,
                                before,
                                crate::lod::triangles(&m)
                            );
                            lods.push(meshes.add(m));
                        }
                        if lods.len() != 3 {
                            lods.truncate(1);
                        }
                    }
                }
                parts.push(Part {
                    mesh: prim.mesh.clone(),
                    material,
                    transform: node.transform,
                    lods,
                });
            }
        }
        prefabs.ready.insert(
            model,
            Prefab {
                parts,
                aabb_min: lo,
                aabb_max: hi,
                surface,
                trunk,
                crown,
            },
        );
    }
    prefabs.loading = still;
}

/// Traces a trunk from the ground up, as circles (height, centre, radius) in model units.
///
/// Each slice of bark is looked at around where the previous slice's centre was, and only
/// the bark close to it counts: so a leaning trunk is followed as it drifts, and branches and
/// roots sticking out do not drag the centre or inflate the radius. Within a slice the centre
/// is the median of the nearby bark, and the radius is the median over angular sectors of the
/// furthest bark in each (a branch on one side only affects a sector or two).
fn trace_trunk(mesh: &Mesh, t: Transform) -> Vec<(f32, Vec2, f32)> {
    use bevy::mesh::VertexAttributeValues as V;
    const SECTORS: usize = 16;
    // Thick enough that even the low-poly twisted trunks have a full ring of bark in each.
    const SLICE: f32 = 0.45;
    const STEP: f32 = 0.25;
    let Some(V::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return Vec::new();
    };
    let pts: Vec<Vec3> = pos
        .iter()
        .map(|p| t.transform_point(Vec3::from(*p)))
        .collect();
    let median = |mut v: Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let mut out = Vec::new();
    let mut c = Vec2::ZERO;
    // Where to look for the trunk in the next slice: anywhere near the origin at first (the
    // lowest slice is below any branches, so a wide look only finds the trunk).
    let mut near = 2.0f32;
    // Up into the crown, until the trunk splits into branches.
    for k in 0..64 {
        let lo = -0.1 + k as f32 * STEP;
        let hi = lo + SLICE;
        let slice: Vec<Vec2> = pts
            .iter()
            .filter(|p| p.y > lo && p.y < hi)
            .map(|p| Vec2::new(p.x, p.z))
            .collect();
        for _ in 0..4 {
            let close: Vec<Vec2> = slice
                .iter()
                .copied()
                .filter(|p| p.distance(c) < near)
                .collect();
            if close.is_empty() {
                break;
            }
            c = Vec2::new(
                median(close.iter().map(|p| p.x).collect()),
                median(close.iter().map(|p| p.y).collect()),
            );
        }
        let mut far = [0.0f32; SECTORS];
        for p in slice.iter().filter(|p| p.distance(c) < near * 1.25) {
            let d = *p - c;
            let a = d.y.atan2(d.x) + std::f32::consts::PI;
            let s = ((a / std::f32::consts::TAU * SECTORS as f32) as usize).min(SECTORS - 1);
            far[s] = far[s].max(d.length());
        }
        let v: Vec<f32> = far.iter().copied().filter(|d| *d > 0.0).collect();
        if v.len() < SECTORS / 3 {
            // Not a whole trunk here (the model starts higher up, or the trunk has ended).
            if out.is_empty() {
                continue;
            }
            break;
        }
        let r = median(v);
        // Lost the trunk to a branch: much thinner, or jumped sideways. Stop here.
        if let Some(&(_, pc, pr)) = out.last()
            && (r < pr * 0.55 || c.distance(pc) > pr * 0.9)
        {
            break;
        }
        out.push(((lo + hi) * 0.5, c, r));
        near = r * 1.6 + 0.2;
    }
    out
}

/// Up to `want` points spread over a leafy mesh's visible surface: random spots on its
/// triangles (by area), dropping any that land on a transparent part of a leaf card and any
/// facing inwards or low down, so berries end up on the outside of the foliage.
fn leaf_surface(mesh: &Mesh, texture: Option<&Image>, want: usize) -> Vec<(Vec3, Vec3)> {
    use bevy::mesh::VertexAttributeValues as V;
    let (Some(V::Float32x3(pos)), Some(V::Float32x3(nor))) = (
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
    ) else {
        return Vec::new();
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(V::Float32x2(uv)) => Some(uv),
        _ => None,
    };
    let idx: Vec<usize> = match mesh.indices() {
        Some(i) => i.iter().collect(),
        None => (0..pos.len()).collect(),
    };
    let (lo, hi) = pos.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(l, h), p| (l.min(Vec3::from(*p)), h.max(Vec3::from(*p))),
    );
    let center = (lo + hi) * 0.5;
    let tris: Vec<[usize; 3]> = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    let areas: Vec<f32> = tris
        .iter()
        .map(|t| {
            let (a, b, c) = (
                Vec3::from(pos[t[0]]),
                Vec3::from(pos[t[1]]),
                Vec3::from(pos[t[2]]),
            );
            (b - a).cross(c - a).length() * 0.5
        })
        .collect();
    let total: f32 = areas.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let alpha = |uv: Vec2| -> f32 {
        let Some(img) = texture else { return 1.0 };
        let Some(data) = img.data.as_ref() else {
            return 1.0;
        };
        let (w, h) = (img.width() as usize, img.height() as usize);
        let bytes = data.len() / (w * h).max(1);
        if bytes < 4 {
            return 1.0;
        }
        let x = ((uv.x.rem_euclid(1.0)) * w as f32) as usize % w;
        let y = ((uv.y.rem_euclid(1.0)) * h as f32) as usize % h;
        data[(y * w + x) * bytes + bytes - 1] as f32 / 255.0
    };
    let mut out = Vec::new();
    let mut k = 0u64;
    while out.len() < want && k < want as u64 * 40 {
        k += 1;
        // Pick a triangle by area, then a point in it.
        let mut r = unit(hash64(k * 3 + 1)) * total;
        let mut ti = 0;
        while ti + 1 < areas.len() && r > areas[ti] {
            r -= areas[ti];
            ti += 1;
        }
        let t = tris[ti];
        let (mut u, mut v) = (unit(hash64(k * 3 + 2)), unit(hash64(k * 3 + 3)));
        if u + v > 1.0 {
            (u, v) = (1.0 - u, 1.0 - v);
        }
        let w = 1.0 - u - v;
        let p = Vec3::from(pos[t[0]]) * w + Vec3::from(pos[t[1]]) * u + Vec3::from(pos[t[2]]) * v;
        if p.y < lo.y + (hi.y - lo.y) * 0.3 {
            continue;
        }
        if let Some(uv) = uvs {
            let q = Vec2::from(uv[t[0]]) * w + Vec2::from(uv[t[1]]) * u + Vec2::from(uv[t[2]]) * v;
            if alpha(q) < 0.6 {
                continue;
            }
        }
        // Leaf cards are double-sided: face whichever way points out of the bush.
        let mut n =
            (Vec3::from(nor[t[0]]) * w + Vec3::from(nor[t[1]]) * u + Vec3::from(nor[t[2]]) * v)
                .normalize_or_zero();
        let out_dir = (p - center).normalize_or_zero();
        if n.dot(out_dir) < 0.0 {
            n = -n;
        }
        // Only the outer shell of the bush, not leaves buried inside it.
        let e = (hi - lo) * 0.5;
        let rel = (p - center) / e.max(Vec3::splat(1e-3));
        if rel.length() < 0.6 || n.dot(out_dir) < 0.2 {
            continue;
        }
        out.push((p, n));
    }
    out
}

/// Which tree model stands where, from the tree's random bits, and how much to scale it:
/// (model, height scale, extra girth). Big enough to close into a roof over the deer.
fn tree_model(kind: u64) -> (Model, f32, f32) {
    let r = unit(kind);
    let i = (hash64(kind) % 5) as u8 + 1;
    let pick2 = if i % 2 == 0 { 1 } else { 3 };
    if r < 0.3 {
        (Model::Pine(i), 2.4, 1.35)
    } else if r < 0.52 {
        (Model::Common(i), 2.6, 1.4)
    } else if r < 0.74 {
        // Thin trunks in the original: thicken them most.
        (Model::Maple(if i % 2 == 0 { 3 } else { 5 }), 2.5, 1.6)
    } else if r < 0.9 {
        (Model::TwistedGreen(pick2), 1.35, 1.25)
    } else if r < 0.925 {
        (Model::Twisted(pick2), 1.25, 1.2)
    } else if r < 0.97 {
        (Model::Dead((i - 1) % 3 + 1), 1.8, 1.4)
    } else {
        (Model::Stump, 2.2, 1.0)
    }
}

fn place_static(
    mut commands: Commands,
    prefabs: Res<Prefabs>,
    world: Res<GameWorld>,
    chunks: Option<Res<Chunks>>,
) {
    let Some(chunks) = chunks else { return };
    if !prefabs.all_loaded() {
        return;
    }
    let w = &world.0;
    let mut shapes = Vec::with_capacity(w.trees.len());
    let mut lod_parts = vec![Vec::new(); CHUNKS * CHUNKS];
    for tree in &w.trees {
        if crate::debug::off("trees") {
            shapes.push(None);
            continue;
        }
        let (chunk, local) = chunks.locate(tree.pos[0], tree.pos[1]);
        let (model, s, girth) = tree_model(tree.kind);
        // Stretched up more than out, lifting the crowns overhead into a roof.
        let scale = Vec3::new(girth, 1.3, girth) * tree.scale * s;
        let t = Transform::from_xyz(local.x, tree.y - 0.15, local.y)
            .with_rotation(Quat::from_rotation_y(tree.yaw))
            .with_scale(scale);
        let (_, parts) = prefabs.spawn_parts(&mut commands, model, chunk, t, true);
        let ci = ((wrap(tree.pos[0]) / CHUNK) as usize).min(CHUNKS - 1)
            + ((wrap(tree.pos[1]) / CHUNK) as usize).min(CHUNKS - 1) * CHUNKS;
        lod_parts[ci].extend(parts);
        let p = &prefabs.ready[&model];
        let root = Vec3::new(tree.pos[0], tree.y - 0.15, tree.pos[1]);
        let mut rings = [Ring::default(); MAX_RINGS];
        let mut ring_count = 0;
        for &(y, c, r) in &p.trunk {
            let y = y * scale.y;
            if ring_count == MAX_RINGS || y > RING_TOP {
                break;
            }
            let o = t.rotation * (Vec3::new(c.x, 0.0, c.y) * scale);
            rings[ring_count] = Ring {
                y: y.max(0.0),
                pos: [wrap(tree.pos[0] + o.x), wrap(tree.pos[1] + o.z)],
                r: (r * scale.x).max(0.2),
            };
            ring_count += 1;
        }
        if ring_count == 0 {
            rings[0] = Ring {
                y: 0.0,
                pos: tree.pos,
                r: tree.radius,
            };
            ring_count = 1;
        }
        shapes.push(Some(TreeShape {
            rings,
            ring_count: ring_count as u8,
            crown_center: p
                .crown
                .map(|(l, h)| root + t.rotation * ((l + h) * 0.5 * scale))
                .unwrap_or(root),
            // Rotation about y: take the wider of x and z so the ellipsoid covers any yaw.
            crown_radii: p
                .crown
                .map(|(l, h)| {
                    let e = (h - l) * 0.5 * scale;
                    let r = e.x.max(e.z) * 0.85;
                    Vec3::new(r, e.y * 0.8, r)
                })
                .unwrap_or(Vec3::ZERO),
        }));
    }
    commands.insert_resource(TreeShapes(shapes));
    commands.insert_resource(TreeLods {
        parts: lod_parts,
        level: vec![0; CHUNKS * CHUNKS],
    });
    for b in &w.boulders {
        let (chunk, local) = chunks.locate(b.pos[0], b.pos[1]);
        let model = Model::Rock((b.kind % 3) as u8 + 1);
        let p = &prefabs.ready[&model];
        // Size the rock so its footprint matches the collision circle, and sink it a little.
        let half = ((p.aabb_max.x - p.aabb_min.x).max(p.aabb_max.z - p.aabb_min.z) * 0.5).max(0.1);
        let s = b.radius / half * 1.1;
        let t = Transform::from_xyz(local.x, b.y - 0.25 * s, local.y)
            .with_rotation(Quat::from_rotation_y(b.yaw))
            .with_scale(Vec3::splat(s));
        prefabs.spawn(&mut commands, model, chunk, t, true);
    }
    commands.insert_resource(VegetationReady);
}

/// Swaps each chunk's trees to the mesh detail its distance calls for. Whole chunks at a
/// time, and only when a chunk crosses a threshold, so almost every frame does nothing.
fn update_lods(
    focus: Res<Focus>,
    mut lods: ResMut<TreeLods>,
    mut parts: Query<(&LodMeshes, &mut Mesh3d)>,
) {
    let f = focus.0;
    let lods = &mut *lods;
    let dists = [
        crate::debug::knob("DEER_LOD1").unwrap_or(LOD_DISTANCES[0]),
        crate::debug::knob("DEER_LOD2").unwrap_or(LOD_DISTANCES[1]),
    ];
    for j in 0..CHUNKS {
        for i in 0..CHUNKS {
            let idx = j * CHUNKS + i;
            let cx = i as f32 * CHUNK + CHUNK * 0.5;
            let cz = j as f32 * CHUNK + CHUNK * 0.5;
            let d = Vec2::new(wrap_delta(cx - f.x), wrap_delta(cz - f.z)).length();
            // A little hysteresis so a chunk on the boundary does not flicker between levels.
            let cur = lods.level[idx];
            let level = dists
                .iter()
                .enumerate()
                .filter(|(k, t)| {
                    let slack = if (*k as u8) < cur { -4.0 } else { 4.0 };
                    d > **t + slack
                })
                .count() as u8;
            if level == cur {
                continue;
            }
            lods.level[idx] = level;
            for &e in &lods.parts[idx] {
                if let Ok((m, mut mesh)) = parts.get_mut(e) {
                    let want = &m.0[(level as usize).min(m.0.len() - 1)];
                    if mesh.0 != *want {
                        mesh.0 = want.clone();
                    }
                }
            }
        }
    }
}

fn update_detail(
    mut commands: Commands,
    focus: Res<Focus>,
    prefabs: Res<Prefabs>,
    world: Res<GameWorld>,
    chunks: Res<Chunks>,
    mut detail: ResMut<DetailChunks>,
) {
    let f = focus.0;
    let radius = detail_radius();
    let mut spawned_this_frame = 0;
    for j in 0..CHUNKS {
        for i in 0..CHUNKS {
            let idx = j * CHUNKS + i;
            let cx = i as f32 * CHUNK + CHUNK * 0.5;
            let cz = j as f32 * CHUNK + CHUNK * 0.5;
            let d = Vec2::new(wrap_delta(cx - f.x), wrap_delta(cz - f.z)).length();
            let have = detail.0.get(&idx).copied();
            if d < radius && have.is_none() && !crate::debug::off("detail") {
                // Spread the work: a couple of chunks per frame.
                if spawned_this_frame >= 2 {
                    continue;
                }
                spawned_this_frame += 1;
                let root = commands
                    .spawn((
                        Detail,
                        Transform::default(),
                        Visibility::default(),
                        ChildOf(chunks.0[idx]),
                    ))
                    .id();
                spawn_detail(&mut commands, &prefabs, &world.0, i, j, root);
                detail.0.insert(idx, root);
            } else if d > radius + 16.0
                && let Some(e) = have
            {
                commands.entity(e).despawn();
                detail.0.remove(&idx);
            }
        }
    }
}

/// Grass in the meadows, ferns and mushrooms under the trees, reeds along the shore and lily
/// pads on the water. The same everywhere for everyone (it comes from the seed), but only drawn
/// near the player.
fn spawn_detail(
    commands: &mut Commands,
    prefabs: &Prefabs,
    w: &World,
    ci: usize,
    cj: usize,
    root: Entity,
) {
    let n = (CHUNK / DETAIL_CELL) as u64;
    let seed = hash64(w.seed ^ 0xDE7A11);
    for b in 0..n {
        for a in 0..n {
            let gi = ci as u64 * n + a;
            let gj = cj as u64 * n + b;
            let h = hash3(seed, gi, gj);
            let lx = (a as f32 + unit(hash64(h ^ 1))) * DETAIL_CELL;
            let lz = (b as f32 + unit(hash64(h ^ 2))) * DETAIL_CELL;
            let x = ci as f32 * CHUNK + lx;
            let z = cj as f32 * CHUNK + lz;
            let r = unit(h);
            let y = w.height(x, z);
            let lake = w.lake_distance(x, z);
            let forest = w.forest_density(x, z);
            let near_tree = {
                let mut near = false;
                w.trees_near(x, z, 1.2, |t| {
                    if deer_shared::world::wrapped_dist2(t.pos, [x, z]) < 1.0 {
                        near = true;
                    }
                });
                near
            };
            let pick = |s: f32| unit(hash64(h ^ 77)) * s;
            let (model, scale, on_water) = if y < WATER_LEVEL {
                // Lily pads on the shallows.
                if lake > -7.0 && y > WATER_LEVEL - 0.9 && r < 0.05 {
                    if r < 0.008 {
                        (Model::LilyFlower, 1.6, true)
                    } else {
                        (
                            Model::Lily(if r < 0.03 { 1 } else { 2 }),
                            1.5 + pick(1.0),
                            true,
                        )
                    }
                } else {
                    continue;
                }
            } else if y < WATER_LEVEL + 0.5 && lake < 6.0 {
                // Reeds along the shore.
                if r < 0.35 {
                    (
                        Model::CatTail((hash64(h) % 3) as u8 + 1),
                        0.8 + pick(0.4),
                        false,
                    )
                } else if r < 0.4 {
                    (Model::Calla, 0.9 + pick(0.3), false)
                } else if r < 0.55 {
                    (Model::GrassTall, 0.45 + pick(0.2), false)
                } else {
                    continue;
                }
            } else if near_tree {
                if r < 0.05 {
                    (Model::Laetiporus, 0.35 + pick(0.2), false)
                } else if r < 0.12 {
                    (Model::Mushroom, 0.35 + pick(0.3), false)
                } else {
                    continue;
                }
            } else if forest > 0.55 {
                match r {
                    r if r < 0.14 => (Model::Fern, 0.5 + pick(0.3), false),
                    r if r < 0.26 => (Model::GrassShort, 0.35 + pick(0.2), false),
                    r if r < 0.28 => (Model::Mushroom, 0.4 + pick(0.3), false),
                    r if r < 0.30 => (
                        Model::Pebble((hash64(h) % 3) as u8 + 1),
                        2.0 + pick(2.0),
                        false,
                    ),
                    r if r < 0.31 => (Model::FlowerBush, 0.5 + pick(0.2), false),
                    _ => continue,
                }
            } else {
                match r {
                    r if r < 0.32 => (Model::GrassShort, 0.35 + pick(0.25), false),
                    r if r < 0.44 => (Model::GrassWispy, 0.4 + pick(0.25), false),
                    r if r < 0.48 => (Model::GrassTall, 0.35 + pick(0.2), false),
                    r if r < 0.51 => (Model::Flower3, 0.22 + pick(0.12), false),
                    r if r < 0.54 => (Model::Flower4, 0.2 + pick(0.1), false),
                    r if r < 0.57 => (Model::Clover, 0.3 + pick(0.15), false),
                    r if r < 0.60 => (Model::Plant7, 1.0 + pick(0.6), false),
                    r if r < 0.61 => (Model::Fern, 0.45 + pick(0.25), false),
                    r if r < 0.62 => (
                        Model::Pebble((hash64(h) % 3) as u8 + 1),
                        2.0 + pick(2.0),
                        false,
                    ),
                    _ => continue,
                }
            };
            let py = if on_water {
                WATER_LEVEL + 0.01
            } else {
                y - 0.03
            };
            let t = Transform::from_xyz(lx, py, lz)
                .with_rotation(Quat::from_rotation_y(
                    unit(hash64(h ^ 9)) * std::f32::consts::TAU,
                ))
                .with_scale(Vec3::splat(scale));
            prefabs.spawn(commands, model, root, t, false);
        }
    }
}
