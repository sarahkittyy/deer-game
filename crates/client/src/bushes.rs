//! Berry bushes. The server owns which berries are left; this draws them. When a bush is eaten
//! bare it withers away and, a little later, a new one grows in its place.

use crate::{
    GameWorld,
    audio::Sfx,
    net::{Net, NetEvent},
    vegetation::{Mat, Model, Prefabs, VegetationReady},
    wrap::{Focus, Wrapped, nearest},
};
use bevy::prelude::*;
use deer_shared::{
    noise::{hash3, unit},
    protocol::BushState,
    world::{MAX_BERRIES, World, wrap_delta},
};

/// How long a bare bush waits before regrowing when there is no server to say so.
const OFFLINE_REGROW: f32 = 25.0;
/// Bushes exist within this distance of the player.
const STREAM_RADIUS: f32 = 110.0;
/// Bushes are this much of the model's size (the model is 1.6 m tall).
const BUSH_SCALE: f32 = 0.72;

#[derive(Resource)]
pub struct Bushes {
    pub states: Vec<BushState>,
    entities: Vec<Option<Entity>>,
    /// Local regrow timers, used only while offline.
    bare_for: Vec<f32>,
}

impl Bushes {
    /// Takes berry `i` off bush `slot` right away (the server confirms or corrects later).
    pub fn eat(&mut self, slot: usize, berry: u8) {
        self.states[slot].berries &= !(1u16 << berry);
    }
}

#[derive(Component)]
pub struct Bush {
    pub slot: usize,
    pub generation: u32,
    /// 0 → 1 while growing in; counts down to 0 while withering away.
    grow: f32,
    dying: bool,
}

#[derive(Component)]
pub struct Berry {
    pub slot: usize,
    pub index: u8,
    /// Which of `BERRIES` grows on this bush.
    pub kind: u8,
}

/// The berries a bush can grow (every berry on one bush is the same): their colour, and the
/// colour they glow when pointed at.
const BERRIES: [(Color, LinearRgba); 4] = [
    // Raspberry
    (
        Color::srgb(0.55, 0.04, 0.10),
        LinearRgba::rgb(9.0, 2.2, 1.6),
    ),
    // Blueberry
    (
        Color::srgb(0.12, 0.17, 0.46),
        LinearRgba::rgb(2.4, 3.6, 9.0),
    ),
    // Blackberry
    (
        Color::srgb(0.16, 0.04, 0.20),
        LinearRgba::rgb(6.0, 2.0, 8.0),
    ),
    // Cloudberry
    (
        Color::srgb(0.86, 0.46, 0.12),
        LinearRgba::rgb(9.0, 5.0, 1.4),
    ),
];

/// The berry mesh and each kind's two looks.
#[derive(Resource, Clone)]
pub struct BerryLook {
    pub mesh: Handle<Mesh>,
    pub normal: Vec<Handle<StandardMaterial>>,
    pub glow: Vec<Handle<StandardMaterial>>,
    offset: Transform,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<PendingWelcome>()
        .add_systems(
            Update,
            (
                reset,
                init.run_if(not(resource_exists::<Bushes>)),
                (apply_welcome, on_net).run_if(resource_exists::<Bushes>),
            )
                .chain(),
        )
        .add_systems(
            Update,
            (offline_regrow, make_berry_look, sync, animate)
                .chain()
                .after(on_net)
                .run_if(resource_exists::<VegetationReady>),
        );
}

/// The berries the server says are left, waiting for the bushes of its world to exist.
#[derive(Resource, Default)]
struct PendingWelcome(Option<Vec<BushState>>);

/// Keeps the server's welcome, and on a new world drops the old bushes (they are made again
/// for the new one by `init`).
fn reset(
    mut commands: Commands,
    mut events: MessageReader<NetEvent>,
    mut pending: ResMut<PendingWelcome>,
    bush_q: Query<Entity, With<Bush>>,
) {
    for ev in events.read() {
        match ev {
            NetEvent::NewWorld => {
                for e in &bush_q {
                    commands.entity(e).despawn();
                }
                commands.remove_resource::<Bushes>();
            }
            NetEvent::Welcome { bushes, .. } => pending.0 = Some(bushes.clone()),
            _ => {}
        }
    }
}

fn apply_welcome(mut pending: ResMut<PendingWelcome>, mut bushes: ResMut<Bushes>) {
    if let Some(states) = pending.0.take_if(|s| s.len() == bushes.states.len()) {
        bushes.states = states;
    }
}

fn full(n: u32) -> u16 {
    ((1u32 << n.min(MAX_BERRIES)) - 1) as u16
}

fn init(mut commands: Commands, world: Res<GameWorld>) {
    let n = world.0.bush_slots.len();
    debug!("{n} bush slots in this world");
    commands.insert_resource(Bushes {
        states: (0..n)
            .map(|s| BushState {
                generation: 0,
                berries: full(world.0.bush(s, 0).berries),
            })
            .collect(),
        entities: vec![None; n],
        bare_for: vec![0.0; n],
    });
}

fn on_net(
    mut commands: Commands,
    mut events: MessageReader<NetEvent>,
    mut bushes: ResMut<Bushes>,
    net: Res<Net>,
    world: Res<GameWorld>,
    focus: Res<Focus>,
) {
    for ev in events.read() {
        match ev {
            NetEvent::Bush { slot, state } => {
                if let Some(s) = bushes.states.get_mut(*slot as usize) {
                    *s = *state;
                }
            }
            NetEvent::Ate { id, slot, berry } => {
                let s = *slot as usize;
                if s < bushes.states.len() {
                    bushes.eat(s, *berry);
                    // Our own munch already played when we clicked.
                    if Some(*id) != net.my_id {
                        let b = world.0.bush(s, bushes.states[s].generation);
                        let p = nearest(Vec3::new(b.pos[0], b.y + 0.8, b.pos[1]), focus.0);
                        commands.trigger(Sfx::Eat(p));
                    }
                }
            }
            _ => {}
        }
    }
}

fn offline_regrow(
    time: Res<Time>,
    net: Res<Net>,
    world: Res<GameWorld>,
    mut bushes: ResMut<Bushes>,
) {
    if net.connected() {
        return;
    }
    let dt = time.delta_secs();
    for slot in 0..bushes.states.len() {
        if bushes.states[slot].berries == 0 {
            bushes.bare_for[slot] += dt;
            if bushes.bare_for[slot] > OFFLINE_REGROW {
                bushes.bare_for[slot] = 0.0;
                let generation = bushes.states[slot].generation + 1;
                bushes.states[slot] = BushState {
                    generation,
                    berries: full(world.0.bush(slot, generation).berries),
                };
            }
        }
    }
}

fn make_berry_look(
    mut commands: Commands,
    look: Option<Res<BerryLook>>,
    prefabs: Res<Prefabs>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if look.is_some() {
        return;
    }
    let Some(berry) = prefabs.ready.get(&Model::Berry) else {
        return;
    };
    let Some(part) = berry.parts.first() else {
        return;
    };
    let Mat::Standard(mat) = &part.material else {
        return;
    };
    let base = materials.get(mat).cloned().unwrap_or_default();
    let (mut normal, mut glow) = (Vec::new(), Vec::new());
    for (color, emissive) in BERRIES {
        let mut n = base.clone();
        n.base_color = color;
        n.perceptual_roughness = 0.35;
        n.reflectance = 0.6;
        let mut g = n.clone();
        g.emissive = emissive;
        normal.push(materials.add(n));
        glow.push(materials.add(g));
    }
    commands.insert_resource(BerryLook {
        mesh: part.mesh.clone(),
        normal,
        glow,
        offset: part.transform,
    });
}

/// Where berry `i` hangs on a bush, in the bush model's own space: one of the points on its
/// visible leaves, a different one per berry, nudged out so it sits on the leaf.
fn berry_spot(
    world: &World,
    slot: usize,
    generation: u32,
    i: u32,
    surface: &[(Vec3, Vec3)],
    taken: &mut Vec<usize>,
) -> Option<Vec3> {
    if surface.is_empty() {
        return None;
    }
    for attempt in 0..32u64 {
        let h = hash3(
            world.seed ^ 0xBE88,
            (slot as u64) << 16 | (i as u64) << 8 | attempt,
            generation as u64,
        );
        let k = (h % surface.len() as u64) as usize;
        let (p, n) = surface[k];
        // Keep berries apart from each other.
        if taken.iter().any(|&t| surface[t].0.distance(p) < 0.18) {
            continue;
        }
        taken.push(k);
        return Some(p + n * 0.05 * unit(h >> 9));
    }
    None
}

fn sync(
    mut commands: Commands,
    world: Res<GameWorld>,
    prefabs: Res<Prefabs>,
    look: Option<Res<BerryLook>>,
    mut bushes: ResMut<Bushes>,
    mut bush_q: Query<&mut Bush>,
    berries: Query<(Entity, &Berry, &ChildOf)>,
    focus: Res<Focus>,
) {
    let Some(look) = look else { return };
    let bushes = &mut *bushes;
    let prefab = &prefabs.ready[&Model::Bush];
    let f = focus.0;
    for slot in 0..bushes.states.len() {
        let state = bushes.states[slot];
        // Only bushes near the player exist (each is a dozen entities with its berries).
        let base = world.0.bush_slots[slot];
        let d = Vec2::new(wrap_delta(base[0] - f.x), wrap_delta(base[1] - f.z)).length();
        if d > STREAM_RADIUS + 10.0 {
            if let Some(e) = bushes.entities[slot].take() {
                commands.entity(e).despawn();
            }
            continue;
        }
        if d > STREAM_RADIUS && bushes.entities[slot].is_none() {
            continue;
        }
        let current =
            bushes.entities[slot].and_then(|e| bush_q.get(e).ok().map(|b| (e, b.generation)));
        if current.is_some_and(|(_, g)| g == state.generation) {
            continue;
        }
        // A new generation: the old bush withers, the new one grows.
        if let Some((e, _)) = current
            && let Ok(mut b) = bush_q.get_mut(e)
        {
            b.dying = true;
        }
        let spawn = world.0.bush(slot, state.generation);
        let p = Vec3::new(spawn.pos[0], spawn.y - 0.05, spawn.pos[1]);
        let root = commands
            .spawn((
                Bush {
                    slot,
                    generation: state.generation,
                    // Bushes there from the start are simply there.
                    grow: if current.is_none() { 1.0 } else { 0.0 },
                    dying: false,
                },
                Wrapped(p),
                Transform::from_translation(p)
                    .with_rotation(Quat::from_rotation_y(spawn.yaw))
                    .with_scale(Vec3::splat(BUSH_SCALE * spawn.scale)),
                Visibility::default(),
            ))
            .id();
        prefabs.spawn(&mut commands, Model::Bush, root, Transform::default(), true);
        let mut taken = Vec::new();
        // From the bush's shared random bits, so everyone sees the same berries.
        let kind = ((spawn.kind >> 12) % BERRIES.len() as u64) as u8;
        for i in 0..spawn.berries.min(MAX_BERRIES) {
            let Some(at) = berry_spot(
                &world.0,
                slot,
                state.generation,
                i,
                &prefab.surface,
                &mut taken,
            ) else {
                continue;
            };
            // Berries keep their real size whatever the bush's scale.
            let s = 1.35 / (BUSH_SCALE * spawn.scale);
            commands.spawn((
                Berry {
                    slot,
                    index: i as u8,
                    kind,
                },
                Mesh3d(look.mesh.clone()),
                MeshMaterial3d(look.normal[kind as usize].clone()),
                Transform::from_translation(at)
                    .with_scale(look.offset.scale * s)
                    .with_rotation(look.offset.rotation),
                ChildOf(root),
            ));
        }
        bushes.entities[slot] = Some(root);
    }
    // Berries that have been eaten go.
    for (e, berry, parent) in &berries {
        let state = bushes.states[berry.slot];
        let Ok(bush) = bush_q.get(parent.0) else {
            continue;
        };
        if bush.generation == state.generation && state.berries & (1 << berry.index) == 0 {
            commands.entity(e).despawn();
        }
    }
}

fn animate(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Bush, &mut Transform)>,
    world: Res<GameWorld>,
) {
    let dt = time.delta_secs();
    for (e, mut b, mut t) in &mut q {
        let full_scale = BUSH_SCALE * world.0.bush(b.slot, b.generation).scale;
        if b.dying {
            b.grow -= dt / 1.5;
            if b.grow <= 0.0 {
                commands.entity(e).despawn();
                continue;
            }
        } else if b.grow < 1.0 {
            b.grow = (b.grow + dt / 4.0).min(1.0);
        } else {
            continue;
        }
        let g = b.grow;
        let eased = g * g * (3.0 - 2.0 * g);
        t.scale = Vec3::splat(full_scale * eased.max(0.001));
    }
}
