//! Life around the player, just for this client: fireflies blinking in the shade, birds circling
//! above the canopy and singing in the trees, rabbits that hop about and bolt from the deer,
//! frogs croaking along the nearest shore, butterflies over the meadows. All of it lives near
//! the player and is replaced as they move on.

use crate::{
    AppState, GameWorld,
    audio::Sfx,
    vegetation::{Model, Prefabs},
    wrap::Focus,
};
use bevy::{
    gltf::Gltf, light::NotShadowCaster, prelude::*, world_serialization::WorldInstanceReady,
};
use deer_shared::world::WATER_LEVEL;
use rand::{Rng, random_range};
use std::f32::consts::TAU;

const FIREFLIES: usize = 70;
const FIREFLY_RANGE: f32 = 28.0;
const RABBITS: usize = 4;
const BUTTERFLIES: usize = 6;
const FROGS: usize = 5;
const BIRD_FLOCKS: usize = 3;

#[derive(Component)]
struct Firefly {
    phase: f32,
    period: f32,
    home: Vec3,
    drift: Vec3,
}

#[derive(Component)]
struct Bird {
    center: Vec3,
    radius: f32,
    speed: f32,
    angle: f32,
    height: f32,
    flap_phase: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Critter {
    Rabbit,
    Frog,
    Butterfly,
}

#[derive(Component)]
struct Animal {
    kind: Critter,
    state: AnimalState,
    timer: f32,
    heading: f32,
    player: Option<Entity>,
    /// Where it is (feet), in the same space as the camera.
    pos: Vec3,
    /// Butterflies wander towards this.
    goal: Vec3,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum AnimalState {
    Idle,
    Hop,
    Flee,
}

struct Clips {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    moving: AnimationNodeIndex,
    run: AnimationNodeIndex,
}

#[derive(Resource)]
struct AnimalAssets {
    rabbit: Handle<Gltf>,
    frog: Handle<Gltf>,
    butterfly: Handle<Gltf>,
    /// Rabbit, frog, butterfly.
    scenes: [Handle<WorldAsset>; 3],
    clips: Option<[Clips; 3]>,
    firefly_mesh: Handle<Mesh>,
    firefly_mat: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct Timers {
    bird_song: f32,
    croak: f32,
}

pub fn plugin(app: &mut App) {
    app.insert_resource(Timers {
        bird_song: 2.0,
        croak: 4.0,
    })
    .add_systems(Startup, load)
    .add_systems(
        Update,
        (
            build_clips,
            keep_fireflies,
            fireflies,
            keep_birds,
            birds,
            keep_animals,
            animals,
            sounds.run_if(in_state(AppState::Playing)),
        )
            .run_if(|| !crate::debug::off("ambient")),
    );
}

fn load(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(AnimalAssets {
        rabbit: assets.load("models/animals/rabbit.glb"),
        frog: assets.load("models/animals/frog.glb"),
        butterfly: assets.load("models/animals/butterfly.glb"),
        scenes: ["rabbit", "frog", "butterfly"].map(|n| {
            assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/animals/{n}.glb")))
        }),
        clips: None,
        firefly_mesh: meshes.add(Sphere::new(0.025).mesh().ico(1).unwrap()),
        firefly_mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 1.0, 0.5),
            emissive: LinearRgba::rgb(60.0, 75.0, 18.0),
            unlit: true,
            ..default()
        }),
    });
}

fn build_clips(
    mut a: ResMut<AnimalAssets>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    if a.clips.is_some() {
        return;
    }
    let (Some(rabbit), Some(frog), Some(fly)) = (
        gltfs.get(&a.rabbit),
        gltfs.get(&a.frog),
        gltfs.get(&a.butterfly),
    ) else {
        return;
    };
    let mut make = |g: &Gltf, idle: &str, moving: &str, run: &str| {
        let clip = |n: &str| g.named_animations.get(n).cloned().unwrap_or_default();
        let mut graph = AnimationGraph::new();
        let root = graph.root;
        let idle = graph.add_clip(clip(idle), 1.0, root);
        let moving = graph.add_clip(clip(moving), 1.0, root);
        let run = graph.add_clip(clip(run), 1.0, root);
        Clips {
            graph: graphs.add(graph),
            idle,
            moving,
            run,
        }
    };
    a.clips = Some([
        make(
            rabbit,
            "Armature|Armature|Sitting.000",
            "Armature|Armature|Jump",
            "Armature|Armature|Running",
        ),
        make(
            frog,
            "FrogArmature|Frog_Idle",
            "FrogArmature|Frog_Jump",
            "FrogArmature|Frog_Jump",
        ),
        make(fly, "Flying", "Flying", "Flying"),
    ]);
}

fn clips_for(a: &AnimalAssets, kind: Critter) -> Option<&Clips> {
    let c = a.clips.as_ref()?;
    Some(match kind {
        Critter::Rabbit => &c[0],
        Critter::Frog => &c[1],
        Critter::Butterfly => &c[2],
    })
}

// ---------------------------------------------------------------- fireflies

fn keep_fireflies(
    mut commands: Commands,
    a: Res<AnimalAssets>,
    focus: Res<Focus>,
    world: Res<GameWorld>,
    mut flies: Query<&mut Firefly>,
) {
    let f = focus.0;
    let mut rng = rand::rng();
    let place = |rng: &mut rand::rngs::ThreadRng| {
        // Mostly in the shade under the trees, never over water.
        for _ in 0..8 {
            let a = rng.random_range(0.0..TAU);
            let r = rng.random_range(3.0..FIREFLY_RANGE);
            let p = Vec3::new(f.x + a.cos() * r, 0.0, f.z + a.sin() * r);
            if world.0.is_water(p.x, p.z) || world.0.forest_density(p.x, p.z) < 0.3 {
                continue;
            }
            return Some(Vec3::new(
                p.x,
                world.0.height(p.x, p.z) + rng.random_range(0.3..2.4),
                p.z,
            ));
        }
        None
    };
    let count = flies.iter().len();
    for _ in count..FIREFLIES {
        if let Some(p) = place(&mut rng) {
            commands.spawn((
                Firefly {
                    phase: rng.random_range(0.0..10.0),
                    period: rng.random_range(2.5..6.0),
                    home: p,
                    drift: Vec3::new(
                        rng.random_range(0.0..10.0),
                        rng.random_range(0.0..10.0),
                        rng.random_range(0.0..10.0),
                    ),
                },
                Mesh3d(a.firefly_mesh.clone()),
                MeshMaterial3d(a.firefly_mat.clone()),
                Transform::from_translation(p),
                NotShadowCaster,
            ));
        }
    }
    for mut fly in &mut flies {
        if Vec2::new(fly.home.x - f.x, fly.home.z - f.z).length() > FIREFLY_RANGE + 4.0
            && let Some(p) = place(&mut rng)
        {
            fly.home = p;
        }
    }
}

fn fireflies(time: Res<Time>, mut q: Query<(&Firefly, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (f, mut tr) in &mut q {
        let d = f.drift;
        let wander = Vec3::new(
            (t * 0.31 + d.x).sin() + (t * 0.17 + d.y).sin() * 0.5,
            (t * 0.23 + d.z).sin() * 0.4,
            (t * 0.27 + d.y).cos() + (t * 0.13 + d.x).cos() * 0.5,
        );
        tr.translation = f.home + wander * 0.8;
        // A slow glow up and down, off most of the time.
        let cycle = ((t + f.phase) / f.period).fract();
        let on = (1.0 - ((cycle - 0.15) / 0.15).abs()).clamp(0.0, 1.0);
        tr.scale = Vec3::splat(on * on * (3.0 - 2.0 * on));
    }
}

// ---------------------------------------------------------------- birds

fn keep_birds(
    mut commands: Commands,
    prefabs: Res<Prefabs>,
    focus: Res<Focus>,
    mut birds: Query<(Entity, &mut Bird)>,
    roots: Query<Entity, With<BirdRoot>>,
) {
    if !prefabs.all_loaded() {
        return;
    }
    let f = focus.0;
    // Flocks wheel over somewhere ahead; ones left far behind fly off and come back near.
    for (_, mut b) in &mut birds {
        if Vec2::new(b.center.x - f.x, b.center.z - f.z).length() > 90.0 {
            b.center = Vec3::new(
                f.x + random_range(-50.0..50.0),
                0.0,
                f.z + random_range(-50.0..50.0),
            );
        }
    }
    if roots.iter().len() > 0 {
        return;
    }
    let root = commands
        .spawn((BirdRoot, Transform::default(), Visibility::default()))
        .id();
    for flock in 0..BIRD_FLOCKS {
        let center = Vec3::new(
            f.x + random_range(-40.0..40.0),
            0.0,
            f.z + random_range(-40.0..40.0),
        );
        let radius = random_range(14.0..30.0);
        let height = random_range(22.0..34.0);
        let speed = random_range(0.25..0.4) * if flock % 2 == 0 { 1.0 } else { -1.0 };
        for i in 0..random_range(3..7) {
            let model = Model::Bird((i % 3 + 1) as u8);
            let e = prefabs.spawn(&mut commands, model, root, Transform::default(), false);
            commands.entity(e).insert(Bird {
                center,
                radius: radius + random_range(-3.0..3.0),
                speed,
                angle: i as f32 * 0.25 + random_range(0.0..0.1),
                height: height + random_range(-2.0..2.0),
                flap_phase: random_range(0.0..TAU),
            });
        }
    }
}

#[derive(Component)]
struct BirdRoot;

fn birds(time: Res<Time>, mut q: Query<(&mut Bird, &mut Transform)>) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for (mut b, mut tr) in &mut q {
        b.angle += b.speed * dt * 12.0 / b.radius;
        let a = b.angle;
        let bob = (t * 1.3 + b.flap_phase).sin() * 0.6;
        tr.translation =
            b.center + Vec3::new(a.cos() * b.radius, b.height + bob, a.sin() * b.radius);
        // Face along the circle, banking into the turn; a quick wingbeat now and then.
        let tangent = Vec3::new(-a.sin(), 0.0, a.cos()) * b.speed.signum();
        let yaw = (-tangent.x).atan2(-tangent.z);
        let flap = ((t * 9.0 + b.flap_phase).sin() * 0.5 + 0.5)
            * ((t * 0.4 + b.flap_phase).sin().max(0.0));
        tr.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_z(-0.35 * b.speed.signum());
        tr.scale = Vec3::new(2.2 + flap * 0.6, 2.2 - flap * 0.4, 2.2);
    }
}

// ---------------------------------------------------------------- rabbits, frogs, butterflies

fn keep_animals(
    mut commands: Commands,
    a: Res<AnimalAssets>,
    focus: Res<Focus>,
    world: Res<GameWorld>,
    q: Query<(Entity, &Animal)>,
) {
    if a.clips.is_none() {
        return;
    }
    let f = focus.0;
    let w = &world.0;
    let mut counts = [0usize; 3];
    for (e, an) in &q {
        let far = Vec2::new(an.pos.x - f.x, an.pos.z - f.z).length();
        let limit = if an.kind == Critter::Frog { 55.0 } else { 60.0 };
        if far > limit {
            commands.entity(e).despawn();
        } else {
            counts[an.kind as usize] += 1;
        }
    }
    let mut rng = rand::rng();
    let mut spawn = |kind: Critter, p: Vec3, rng: &mut rand::rngs::ThreadRng| {
        let (scene, scale) = match kind {
            Critter::Rabbit => (&a.scenes[0], 1.0),
            Critter::Frog => (&a.scenes[1], 1.5),
            Critter::Butterfly => (&a.scenes[2], 1.6),
        };
        commands
            .spawn((
                Animal {
                    kind,
                    state: AnimalState::Idle,
                    timer: rng.random_range(0.5..4.0),
                    heading: rng.random_range(0.0..TAU),
                    player: None,
                    pos: p,
                    goal: p,
                },
                WorldAssetRoot(scene.clone()),
                Transform::from_translation(p).with_scale(Vec3::splat(scale)),
            ))
            .observe(animal_ready);
    };
    // Rabbits on dry ground somewhere around, out of sight-ish.
    for _ in counts[0]..RABBITS {
        let ang = rng.random_range(0.0..TAU);
        let r = rng.random_range(16.0..40.0);
        let (x, z) = (f.x + ang.cos() * r, f.z + ang.sin() * r);
        if w.height(x, z) > WATER_LEVEL + 0.5 {
            spawn(Critter::Rabbit, Vec3::new(x, w.height(x, z), z), &mut rng);
        }
    }
    // Butterflies over the meadows.
    for _ in counts[2]..BUTTERFLIES {
        let ang = rng.random_range(0.0..TAU);
        let r = rng.random_range(6.0..30.0);
        let (x, z) = (f.x + ang.cos() * r, f.z + ang.sin() * r);
        if w.forest_density(x, z) < 0.45 && !w.is_water(x, z) {
            spawn(
                Critter::Butterfly,
                Vec3::new(x, w.height(x, z) + 0.8, z),
                &mut rng,
            );
        }
    }
    // Frogs sit on the shore of a lake near the player.
    if counts[1] < FROGS {
        for lake in &w.lakes {
            let c = crate::wrap::nearest(Vec3::new(lake.center[0], 0.0, lake.center[1]), f);
            if Vec2::new(c.x - f.x, c.z - f.z).length() > lake.radius + 35.0 {
                continue;
            }
            for _ in counts[1]..FROGS {
                for _ in 0..10 {
                    let ang = rng.random_range(0.0..TAU);
                    let r = lake.radius * rng.random_range(0.95..1.3);
                    let (x, z) = (c.x + ang.cos() * r, c.z + ang.sin() * r);
                    let h = w.height(x, z);
                    if h > WATER_LEVEL + 0.02 && h < WATER_LEVEL + 0.6 {
                        spawn(Critter::Frog, Vec3::new(x, h, z), &mut rng);
                        break;
                    }
                }
            }
            break;
        }
    }
}

fn animal_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    a: Res<AnimalAssets>,
    children: Query<&Children>,
    mut animals: Query<&mut Animal>,
    mut players: Query<&mut AnimationPlayer>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(mut an) = animals.get_mut(ready.entity) else {
        return;
    };
    let Some(clips) = clips_for(&a, an.kind) else {
        return;
    };
    for e in children.iter_descendants(ready.entity) {
        commands.entity(e).insert(NotShadowCaster);
        if let Ok(m) = mesh_materials.get(e)
            && let Some(mut mat) = materials.get_mut(&m.0)
            && mat.metallic > 0.0
        {
            mat.metallic = 0.0;
            mat.perceptual_roughness = 0.7;
        }
        if let Ok(mut p) = players.get_mut(e) {
            p.play(clips.idle).repeat();
            commands
                .entity(e)
                .insert(AnimationGraphHandle(clips.graph.clone()));
            an.player = Some(e);
        }
    }
}

fn animals(
    mut commands: Commands,
    time: Res<Time>,
    a: Res<AnimalAssets>,
    world: Res<GameWorld>,
    deer: Query<&Transform, (With<crate::player::LocalDeer>, Without<Animal>)>,
    mut q: Query<(&mut Animal, &mut Transform)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let deer = deer.single().ok().map(|t| t.translation);
    let w = &world.0;
    let mut rng = rand::rng();
    for (mut an, mut tr) in &mut q {
        let an = &mut *an;
        let Some(clips) = clips_for(&a, an.kind) else {
            continue;
        };
        let prev = an.state;
        an.timer -= dt;
        let deer_close = deer.is_some_and(|d| d.distance(an.pos) < 6.0);

        match an.kind {
            Critter::Butterfly => {
                // Flutter between nearby goals, bobbing.
                if an.pos.distance(an.goal) < 0.5 || an.timer <= 0.0 {
                    let g = an.pos
                        + Vec3::new(
                            rng.random_range(-4.0..4.0),
                            0.0,
                            rng.random_range(-4.0..4.0),
                        );
                    an.goal = Vec3::new(g.x, w.height(g.x, g.z) + rng.random_range(0.5..1.6), g.z);
                    an.timer = rng.random_range(2.0..5.0);
                }
                let to = an.goal - an.pos;
                let step = to.normalize_or_zero() * 1.2 * dt;
                let bob = (t * 6.0 + an.heading).sin() * 0.01;
                an.pos += step + Vec3::Y * bob;
                if step.length_squared() > 0.0 {
                    an.heading = (-to.x).atan2(-to.z);
                }
            }
            Critter::Rabbit | Critter::Frog => {
                if deer_close && an.state != AnimalState::Flee {
                    an.state = AnimalState::Flee;
                    an.timer = if an.kind == Critter::Frog { 0.7 } else { 2.5 };
                    if let Some(d) = deer {
                        let away = an.pos - d;
                        an.heading = (-away.x).atan2(-away.z);
                    }
                    commands.trigger(Sfx::Rustle(an.pos));
                } else if an.timer <= 0.0 {
                    if an.state == AnimalState::Idle {
                        an.state = AnimalState::Hop;
                        an.timer = if an.kind == Critter::Frog { 0.6 } else { 0.55 };
                        an.heading += rng.random_range(-1.2..1.2);
                    } else {
                        an.state = AnimalState::Idle;
                        an.timer = rng.random_range(1.5..6.0);
                    }
                }
                let speed = match (an.kind, an.state) {
                    (_, AnimalState::Idle) => 0.0,
                    (Critter::Rabbit, AnimalState::Hop) => 1.2,
                    (Critter::Rabbit, _) => 5.0,
                    (_, AnimalState::Hop) => 0.6,
                    _ => 1.4,
                };
                let dir = Vec3::new(-an.heading.sin(), 0.0, -an.heading.cos());
                let next = an.pos + dir * speed * dt;
                // Rabbits keep out of the water; frogs may hop in, and disappear when they do.
                if w.is_water(next.x, next.z) {
                    if an.kind == Critter::Frog && an.state == AnimalState::Flee {
                        an.pos = Vec3::new(next.x, -5.0, next.z);
                    } else {
                        an.heading += 2.5;
                    }
                } else {
                    an.pos = Vec3::new(next.x, w.height(next.x, next.z), next.z);
                }
            }
        }
        tr.translation = an.pos;
        tr.rotation = Quat::from_rotation_y(an.heading + std::f32::consts::PI);
        if an.kind == Critter::Butterfly {
            tr.rotation = Quat::from_rotation_y(an.heading);
        }
        if an.state != prev
            && let Some(p) = an.player
            && let Ok(mut player) = players.get_mut(p)
        {
            let node = match an.state {
                AnimalState::Idle => clips.idle,
                AnimalState::Hop => clips.moving,
                AnimalState::Flee => clips.run,
            };
            player.stop_all();
            player.play(node).repeat();
        }
    }
}

// ---------------------------------------------------------------- sounds

fn sounds(
    mut commands: Commands,
    time: Res<Time>,
    mut timers: ResMut<Timers>,
    focus: Res<Focus>,
    world: Res<GameWorld>,
    frogs: Query<&Animal>,
) {
    let dt = time.delta_secs();
    let f = focus.0;
    let w = &world.0;
    timers.bird_song -= dt;
    if timers.bird_song <= 0.0 {
        timers.bird_song = random_range(1.5..6.0);
        // Up in a tree somewhere nearby.
        let a = random_range(0.0..TAU);
        let r = random_range(6.0..30.0);
        let (x, z) = (f.x + a.cos() * r, f.z + a.sin() * r);
        commands.trigger(Sfx::Bird(Vec3::new(
            x,
            w.height(x, z) + random_range(6.0..14.0),
            z,
        )));
    }
    timers.croak -= dt;
    if timers.croak <= 0.0 {
        timers.croak = random_range(1.5..5.0);
        let sitting: Vec<Vec3> = frogs
            .iter()
            .filter(|a| a.kind == Critter::Frog && a.pos.y > -1.0)
            .map(|a| a.pos)
            .collect();
        if !sitting.is_empty() {
            let p = sitting[random_range(0..sitting.len())];
            commands.trigger(Sfx::Frog(p));
        }
    }
}
