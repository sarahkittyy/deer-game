//! Wolves as everyone sees them. The server runs them (`crates/server/src/wolf.rs`); here they
//! are drawn in the present rather than a snapshot behind like ghosts: each wolf is carried on
//! with its last velocity, and the gap to where the newest snapshot says it is now is worked
//! off over a few frames, so it moves smoothly and does not trail what it is chasing. A wolf
//! chasing this player heads for where the player's deer is now, not where the server last
//! saw it. The pounce is played out here from one message, and lands on the deer's own game.

use crate::{
    AppState, GameEntity, GameWorld,
    audio::Sfx,
    ghosts::Ghost,
    knock::Knock,
    net::{Net, NetEvent},
    player::LocalDeer,
    wrap::{Focus, Wrapped, nearest},
};
use bevy::{app::AnimationSystems, prelude::*, world_serialization::WorldInstanceReady};
use deer_shared::{
    protocol::{
        WolfCall, WolfMood,
        wolf::{HIT_AT, LEAP_SECS},
    },
    world::{wrap, wrap_delta},
};
use std::f32::consts::{PI, TAU};

/// The wolf model is ~2.7 units at the head and 5.5 long with its tail: about 1 m and 2 m.
const MODEL_SCALE: f32 = 0.37;
/// Speeds (m/s) the walk and gallop clips are authored for at this scale.
const WALK_CLIP_SPEED: f32 = 1.3;
const GALLOP_CLIP_SPEED: f32 = 5.0;
/// Snapshots are this old by the time they arrive and are acted on, roughly.
const LEAD: f32 = 0.08;
/// How fast the gap to the server's position closes (1/s).
const CATCH_UP: f32 = 5.0;
/// Further off than this, the wolf jumps straight to where the server has it.
const SNAP: f32 = 8.0;
/// Height of the pounce's arc.
const LEAP_HEIGHT: f32 = 0.9;
/// One gallop stride, and where in it each paw lands: two pairs, "ba-dum ... ba-dum".
const STRIDE: f32 = 2.4;
const FOOTFALLS: [f32; 4] = [0.0, 0.11, 0.45, 0.56];

/// Clip indices in wolf.glb (the clips without the `AnimalArmature|` prefix).
mod clip {
    pub const GALLOP: usize = 3;
    pub const GALLOP_JUMP: usize = 4;
    pub const WALK: usize = 8;
    pub const IDLE_HEADLOW: usize = 9;
    pub const IDLE: usize = 11;
}
const CLIPS: [usize; 5] = [
    clip::IDLE,
    clip::WALK,
    clip::GALLOP,
    clip::GALLOP_JUMP,
    clip::IDLE_HEADLOW,
];
const IDLE: usize = 0;
const WALK: usize = 1;
const GALLOP: usize = 2;
const LEAP: usize = 3;
const SNIFF: usize = 4;

#[derive(Resource)]
struct WolfAssets {
    scene: Handle<WorldAsset>,
    graph: Handle<AnimationGraph>,
    nodes: [AnimationNodeIndex; 5],
}

struct Leap {
    from: Vec2,
    dir: Vec2,
    dist: f32,
    t: f32,
    victim: u32,
    hit: bool,
}

#[derive(Component)]
pub struct Wolf {
    pub id: u8,
    /// Where it is drawn (wrapped x, z).
    pos: Vec2,
    vel: Vec2,
    /// Where the server has it now, less where it is drawn: worked off a little each frame.
    err: Vec2,
    yaw: f32,
    server_yaw: f32,
    mood: WolfMood,
    target: Option<u32>,
    leap: Option<Leap>,
    /// Distance walked since the last paw sound; gallop stride phase (0..1).
    walked: f32,
    stride: f32,
    /// Counts down through a bark's toss of the head.
    bark: f32,
    /// Speed it is drawn moving at, for the animation.
    speed: f32,
    /// Where the head should look, in the wolf's own space: (yaw, pitch).
    look: Vec2,
}

/// Blend state and the bones the head turn moves, once the model has spawned.
#[derive(Component)]
struct WolfRig {
    player: Entity,
    weights: [f32; 5],
    neck: [Entity; 4],
    neck_ancestors: Vec<Entity>,
    look: Vec2,
    leaping: bool,
}

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, load)
        .add_systems(
            Update,
            (on_net, drive, blend)
                .chain()
                .after(crate::player::PlayerSet)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            PostUpdate,
            turn_head
                .after(AnimationSystems)
                .before(TransformSystems::Propagate),
        );
}

fn load(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let path = "models/wolf.glb";
    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let nodes = CLIPS.map(|i| {
        graph.add_clip(
            assets.load(GltfAssetLabel::Animation(i).from_asset(path)),
            1.0,
            root,
        )
    });
    commands.insert_resource(WolfAssets {
        scene: assets.load(GltfAssetLabel::Scene(0).from_asset(path)),
        graph: graphs.add(graph),
        nodes,
    });
}

fn spawn(commands: &mut Commands, assets: &WolfAssets, id: u8, pos: Vec3, yaw: f32) {
    let root = commands
        .spawn((
            Wolf {
                id,
                pos: Vec2::new(pos.x, pos.z),
                vel: Vec2::ZERO,
                err: Vec2::ZERO,
                yaw,
                server_yaw: yaw,
                mood: WolfMood::Wander,
                target: None,
                leap: None,
                walked: 0.0,
                stride: 0.0,
                bark: 0.0,
                speed: 0.0,
                look: Vec2::ZERO,
            },
            GameEntity,
            Wrapped(pos),
            Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
        ))
        .id();
    commands
        .spawn((
            WorldAssetRoot(assets.scene.clone()),
            Transform::from_scale(Vec3::splat(MODEL_SCALE))
                .with_rotation(Quat::from_rotation_y(PI)),
            ChildOf(root),
        ))
        .observe(on_model_ready);
}

fn on_model_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    assets: Res<WolfAssets>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut players: Query<&mut AnimationPlayer>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let model_root = ready.entity;
    let Ok(&ChildOf(wolf)) = parents.get(model_root) else {
        return;
    };
    let mut player_entity = None;
    let mut neck: [Option<Entity>; 4] = [None; 4];
    for e in children.iter_descendants(model_root) {
        // Same pack as the deer: matte fur instead of the FBX's metal.
        if let Ok(m) = mesh_materials.get(e)
            && let Some(mut mat) = materials.get_mut(&m.0)
            && mat.metallic > 0.0
        {
            mat.metallic = 0.0;
            mat.perceptual_roughness = 0.85;
            mat.reflectance = 0.35;
        }
        if players.contains(e) {
            player_entity = Some(e);
        }
        if let Ok(name) = names.get(e) {
            match name.as_str() {
                "Neck1" => neck[0] = Some(e),
                "Neck2" => neck[1] = Some(e),
                "Neck3" => neck[2] = Some(e),
                "Head" => neck[3] = Some(e),
                _ => {}
            }
        }
    }
    let (Some(player_entity), [Some(n1), Some(n2), Some(n3), Some(head)]) = (player_entity, neck)
    else {
        warn!("wolf model is missing its animation player or neck bones");
        return;
    };
    let mut neck_ancestors = Vec::new();
    let mut e = n1;
    while let Ok(&ChildOf(p)) = parents.get(e) {
        if p == wolf {
            break;
        }
        neck_ancestors.push(p);
        e = p;
    }
    neck_ancestors.reverse();

    let mut player = players.get_mut(player_entity).unwrap();
    for (i, node) in assets.nodes.iter().enumerate() {
        let a = player.play(*node);
        a.set_weight(1e-3);
        if i == LEAP {
            a.set_repeat(bevy::animation::RepeatAnimation::Never);
        } else {
            a.repeat();
        }
    }
    commands
        .entity(player_entity)
        .insert(AnimationGraphHandle(assets.graph.clone()));
    commands.entity(wolf).insert(WolfRig {
        player: player_entity,
        weights: [0.0; 5],
        neck: [n1, n2, n3, head],
        neck_ancestors,
        look: Vec2::ZERO,
        leaping: false,
    });
}

/// Where a deer is, wrapped: this player's own (now) or a ghost (as drawn).
fn deer_at(
    id: u32,
    net: &Net,
    me: &Query<&LocalDeer>,
    ghosts: &Query<(&Ghost, &Wrapped), Without<Wolf>>,
) -> Option<Vec3> {
    if Some(id) == net.my_id {
        return me
            .single()
            .ok()
            .map(|d| Vec3::new(wrap(d.pos.x), d.pos.y, wrap(d.pos.z)));
    }
    ghosts.iter().find(|(g, _)| g.id == id).map(|(_, w)| w.0)
}

fn on_net(
    mut commands: Commands,
    mut events: MessageReader<NetEvent>,
    assets: Res<WolfAssets>,
    net: Res<Net>,
    focus: Res<Focus>,
    me: Query<&LocalDeer>,
    ghosts: Query<(&Ghost, &Wrapped), Without<Wolf>>,
    mut wolves: Query<(Entity, &mut Wolf, &Wrapped)>,
) {
    // A long frame can bring several snapshots at once, and a wolf spawned for the first is not
    // in the query yet: without this, each later one would spawn another copy of it.
    let mut spawned = std::collections::HashSet::new();
    for ev in events.read() {
        match ev {
            NetEvent::Disconnected | NetEvent::Welcome { .. } | NetEvent::NewWorld => {
                for (e, ..) in &wolves {
                    commands.entity(e).despawn();
                }
            }
            NetEvent::Wolves(states) => {
                for (i, s) in states.iter().enumerate() {
                    let i = i as u8;
                    let Some((_, mut w, _)) = wolves.iter_mut().find(|(_, w, _)| w.id == i) else {
                        if spawned.insert(i) {
                            spawn(&mut commands, &assets, i, Vec3::from_array(s.pos), s.yaw);
                        }
                        continue;
                    };
                    w.vel = Vec2::from_array(s.vel);
                    w.server_yaw = s.yaw;
                    w.mood = s.mood;
                    w.target = s.target;
                    if w.leap.is_some() {
                        continue;
                    }
                    let now = Vec2::new(s.pos[0], s.pos[2]) + w.vel * LEAD;
                    let gap = Vec2::new(wrap_delta(now.x - w.pos.x), wrap_delta(now.y - w.pos.y));
                    if gap.length() > SNAP {
                        w.pos = now;
                        w.err = Vec2::ZERO;
                    } else {
                        w.err = gap;
                    }
                }
            }
            NetEvent::WolfCall { wolf, call } => {
                if let Some((_, mut w, at)) = wolves.iter_mut().find(|(_, w, _)| w.id == *wolf) {
                    let p = nearest(at.0, focus.0) + Vec3::Y * 0.7;
                    commands.trigger(match call {
                        WolfCall::Bark => Sfx::WolfBark(p),
                        WolfCall::Growl => Sfx::WolfGrowl(p),
                    });
                    if *call == WolfCall::Bark {
                        w.bark = 0.35;
                    }
                }
            }
            NetEvent::Pounce {
                wolf,
                victim,
                from,
                dir,
            } => {
                let Some((_, mut w, at)) = wolves.iter_mut().find(|(_, w, _)| w.id == *wolf) else {
                    continue;
                };
                // Leap from where this wolf is drawn, at the deer where it is drawn here: on the
                // victim's own screen that is where the deer really is.
                let server_from = Vec2::new(from[0], from[2]);
                let mut start = w.pos;
                let off = Vec2::new(
                    wrap_delta(server_from.x - start.x),
                    wrap_delta(server_from.y - start.y),
                );
                if off.length() > 5.0 {
                    start = server_from;
                }
                let mut d = Vec2::from_array(*dir).normalize_or(Vec2::NEG_Y);
                let mut dist = 2.5;
                if let Some(v) = deer_at(*victim, &net, &me, &ghosts) {
                    let to = Vec2::new(wrap_delta(v.x - start.x), wrap_delta(v.z - start.y));
                    if to.length() > 0.1 {
                        d = to.normalize();
                        // Too far to reach (lag): start the leap closer.
                        if to.length() > 4.0 {
                            start += d * (to.length() - 2.5);
                        }
                        dist = to.length().min(2.5) + 1.2;
                    }
                }
                w.leap = Some(Leap {
                    from: start,
                    dir: d,
                    dist,
                    t: 0.0,
                    victim: *victim,
                    hit: false,
                });
                w.err = Vec2::ZERO;
                commands.trigger(Sfx::WolfAttack(nearest(at.0, focus.0) + Vec3::Y * 0.7));
            }
            _ => {}
        }
    }
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + PI).rem_euclid(TAU) - PI
}

/// Moves each wolf on: the pounce, or prediction and catching up with the server.
fn drive(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<GameWorld>,
    net: Res<Net>,
    focus: Res<Focus>,
    me: Query<&LocalDeer>,
    ghosts: Query<(&Ghost, &Wrapped), Without<Wolf>>,
    mut wolves: Query<(&mut Wolf, &mut Wrapped, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.05);
    let w = &world.0;
    for (mut wolf, mut wrapped, mut t) in &mut wolves {
        let before = wolf.pos;
        let mut lift = 0.0;
        if let Some(leap) = &mut wolf.leap {
            leap.t += dt;
            let f = (leap.t / LEAP_SECS).min(1.0);
            let pos = leap.from + leap.dir * leap.dist * (f * (2.0 - f));
            lift = 4.0 * f * (1.0 - f) * LEAP_HEIGHT;
            let (dir, victim, hit_now) = (leap.dir, leap.victim, !leap.hit && leap.t >= HIT_AT);
            if hit_now {
                leap.hit = true;
            }
            let done = f >= 1.0;
            wolf.pos = pos;
            wolf.yaw = (-dir.x).atan2(-dir.y);
            if hit_now {
                if Some(victim) == net.my_id {
                    commands.trigger(Knock { dir });
                } else if let Some(v) = deer_at(victim, &net, &me, &ghosts) {
                    commands.trigger(Sfx::Thud(nearest(v, focus.0) + Vec3::Y * 0.7, 1.0));
                }
            }
            if done {
                wolf.leap = None;
                wolf.vel = Vec2::ZERO;
            }
        } else {
            let mut vel = wolf.vel;
            // Chasing this player: aim at where the deer is now.
            if wolf.mood == WolfMood::Chase
                && wolf.target.is_some()
                && wolf.target == net.my_id
                && let Some(v) = wolf.target.and_then(|id| deer_at(id, &net, &me, &ghosts))
            {
                let to = Vec2::new(wrap_delta(v.x - wolf.pos.x), wrap_delta(v.z - wolf.pos.y));
                if to.length() > 0.5 {
                    vel = to.normalize() * vel.length();
                }
            }
            let fix = wolf.err * (1.0 - (-dt * CATCH_UP).exp());
            wolf.err -= fix;
            wolf.pos += vel * dt + fix;
            let speed = vel.length();
            let want = if speed > 0.4 && wolf.mood != WolfMood::Stare {
                (-vel.x).atan2(-vel.y)
            } else {
                wolf.server_yaw
            };
            let max = if speed > 4.0 { 8.0 } else { 4.5 } * dt;
            wolf.yaw = (wolf.yaw + angle_diff(want, wolf.yaw).clamp(-max, max)).rem_euclid(TAU);
        }
        wolf.pos = Vec2::new(wrap(wolf.pos.x), wrap(wolf.pos.y));
        let moved = Vec2::new(
            wrap_delta(wolf.pos.x - before.x),
            wrap_delta(wolf.pos.y - before.y),
        )
        .length();
        wolf.speed = if dt > 0.0 { moved / dt } else { 0.0 };

        let ground = w.height(wolf.pos.x, wolf.pos.y);
        wrapped.0 = Vec3::new(wolf.pos.x, ground + lift, wolf.pos.y);
        // Lean with the slope it is on.
        let fwd = Vec2::new(-wolf.yaw.sin(), -wolf.yaw.cos());
        let (a, b) = (wolf.pos + fwd * 0.6, wolf.pos - fwd * 0.6);
        let slope = (w.height(a.x, a.y) - w.height(b.x, b.y))
            .atan2(1.2)
            .clamp(-0.4, 0.4);
        t.rotation = Quat::from_rotation_y(wolf.yaw) * Quat::from_rotation_x(slope);

        // Paws: a soft step every so often at a walk, a gallop's pattern at a run.
        let at = nearest(wrapped.0, focus.0);
        if lift == 0.0 {
            if wolf.speed > 3.5 {
                let old = wolf.stride;
                wolf.stride += moved / STRIDE;
                for f in FOOTFALLS {
                    let (o, n) = (old - f, wolf.stride - f);
                    if o.floor() != n.floor() {
                        commands.trigger(Sfx::WolfStep(at, true));
                    }
                }
                wolf.stride = wolf.stride.fract();
            } else {
                wolf.walked += moved;
                if wolf.walked > 0.45 {
                    wolf.walked = 0.0;
                    commands.trigger(Sfx::WolfStep(at, false));
                }
            }
        }

        // The head: on its deer while after one, a bark's toss on top, low while stalking.
        let mut look = Vec2::ZERO;
        if let Some(v) = wolf.target.and_then(|id| deer_at(id, &net, &me, &ghosts)) {
            let to = Vec3::new(
                wrap_delta(v.x - wolf.pos.x),
                v.y + 0.9 - (ground + 0.7),
                wrap_delta(v.z - wolf.pos.y),
            );
            let flat = Vec2::new(to.x, to.z).length();
            let yaw = angle_diff((-to.x).atan2(-to.z), wolf.yaw);
            look = Vec2::new(yaw.clamp(-1.1, 1.1), to.y.atan2(flat).clamp(-0.5, 0.5));
        }
        if wolf.mood == WolfMood::Stalk {
            look.y -= 0.3;
        }
        if wolf.bark > 0.0 {
            wolf.bark -= dt;
            let k = (wolf.bark / 0.35).clamp(0.0, 1.0);
            look.y += (k * PI).sin() * 0.45;
        }
        wolf.look = look;
    }
}

fn blend(
    time: Res<Time>,
    mut wolves: Query<(&Wolf, &mut WolfRig)>,
    mut players: Query<&mut AnimationPlayer>,
    assets: Res<WolfAssets>,
) {
    let dt = time.delta_secs();
    for (wolf, mut rig) in &mut wolves {
        let Ok(mut player) = players.get_mut(rig.player) else {
            continue;
        };
        let s = wolf.speed;
        let mut target = [0.0f32; 5];
        let leaping = wolf.leap.is_some();
        if leaping {
            target[LEAP] = 1.0;
            if !rig.leaping
                && let Some(a) = player.animation_mut(assets.nodes[LEAP])
            {
                a.replay();
                a.seek_to(0.15);
                a.set_speed(0.75 / LEAP_SECS);
            }
        } else {
            let walk_t = (s / 1.0).clamp(0.0, 1.0);
            let run_t = ((s - 2.0) / 2.5).clamp(0.0, 1.0);
            // Standing about while wandering, it sniffs the ground; after a deer, it is alert.
            let idle = if wolf.mood == WolfMood::Wander {
                SNIFF
            } else {
                IDLE
            };
            target[idle] = 1.0 - walk_t;
            target[WALK] = walk_t * (1.0 - run_t);
            target[GALLOP] = walk_t * run_t;
        }
        rig.leaping = leaping;
        target[IDLE] = target[IDLE].max(0.02);
        let rate = 1.0 - (-dt * if leaping { 18.0 } else { 8.0 }).exp();
        for i in 0..5 {
            rig.weights[i] += (target[i] - rig.weights[i]) * rate;
            if let Some(a) = player.animation_mut(assets.nodes[i]) {
                a.set_weight(rig.weights[i].max(1e-3));
            }
        }
        if let Some(a) = player.animation_mut(assets.nodes[WALK]) {
            a.set_speed((s / WALK_CLIP_SPEED).clamp(0.5, 1.8));
        }
        if let Some(a) = player.animation_mut(assets.nodes[GALLOP]) {
            a.set_speed((s / GALLOP_CLIP_SPEED).clamp(0.7, 2.0));
        }
    }
}

/// Turns the neck and head towards `Wolf::look`, split across the bones like the deer's.
fn turn_head(
    time: Res<Time>,
    mut wolves: Query<(&Wolf, &mut WolfRig)>,
    mut transforms: Query<&mut Transform>,
) {
    const SHARE: [f32; 4] = [0.3, 0.25, 0.2, 0.25];
    let rate = 1.0 - (-time.delta_secs() * 8.0).exp();
    for (wolf, mut rig) in &mut wolves {
        let look = rig.look + (wolf.look - rig.look) * rate;
        rig.look = look;
        let mut parent = Quat::IDENTITY;
        for &a in &rig.neck_ancestors {
            if let Ok(t) = transforms.get(a) {
                parent *= t.rotation;
            }
        }
        for (k, &bone) in rig.neck.iter().enumerate() {
            let Ok(mut t) = transforms.get_mut(bone) else {
                break;
            };
            let delta =
                Quat::from_rotation_y(look.x * SHARE[k]) * Quat::from_rotation_x(look.y * SHARE[k]);
            let local = (parent.inverse() * delta * parent * t.rotation).normalize();
            if local.is_finite() {
                t.rotation = local;
            }
            parent *= t.rotation;
        }
    }
}
