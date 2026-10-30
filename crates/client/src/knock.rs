//! Knocked flying by a wolf. The player's deer leaves the ground away from the wolf at a
//! somewhat random angle (far and low, or high, but never up into the leaves), tumbling end over
//! end; it bounces off trunks at whatever height it meets them, and off the ground, slides to a
//! stop, lies a moment however it landed, and pops back up. Other players see the tumble through
//! `Pose`.
//!
//! The whole body spins about its middle, with the death clip held part-way for a limp pose;
//! on top of that every joint of the neck, spine, legs, tail and ears is a little spring that
//! lags behind the body's turns and hangs with gravity (`flop`), so it all whips about. That
//! part runs on every client from the motion it sees, for ghosts too.

use crate::{
    GameWorld,
    audio::Sfx,
    deer::{DeerAnim, Reach},
    player::LocalDeer,
    vegetation::{Ring, TreeShapes},
    wrap::Focus,
};
use bevy::prelude::*;
use deer_shared::{
    protocol::Action,
    world::{WADE_DEPTH, WATER_LEVEL, World, wrap, wrap_delta},
};

/// The body's middle, above the feet: the deer tumbles about it.
pub const CENTER: f32 = 0.7;
/// Radius of the ball that stands in for the body against trunks and the ground.
const BODY: f32 = 0.45;
const GRAVITY: f32 = 16.0;
/// Launch speed (m/s), picked at random in this range.
const LAUNCH: (f32, f32) = (15.0, 26.0);
/// How far either side of straight away from the wolf the deer may fly (radians).
const SPREAD: f32 = 0.45;
/// Lying still after landing, then popping back up, seconds.
const LIE_SECS: f32 = 1.2;
const GET_UP_SECS: f32 = 0.5;
/// How high it hops getting up.
const POP_HOP: f32 = 0.35;
/// The legs' pose in the air: a little way into the collapse, limp.
const AIR_FALL: f32 = 0.35;
/// Bounciness of trunks and the ground.
const TRUNK_BOUNCE: f32 = 0.4;
const GROUND_BOUNCE: f32 = 0.3;

/// Knock the player's deer flying along `dir` (horizontal, wrapped world x and z).
#[derive(Event, Clone, Copy, Debug)]
pub struct Knock {
    pub dir: Vec2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Flying,
    Lying,
    GettingUp,
}

/// On the player's deer while it is knocked over; its controls do nothing meanwhile.
#[derive(Component)]
pub struct Knocked {
    /// The whole body's rotation, about its middle.
    pub rot: Quat,
    spin: Vec3,
    pub phase: Phase,
    /// Seconds in the current phase.
    t: f32,
    pub fall: f32,
    /// Which way it faces once up again.
    yaw: f32,
    since_thud: f32,
    /// How it lay, and its feet's height, as it started getting up.
    from: Quat,
    from_y: f32,
}

pub fn plugin(app: &mut App) {
    if std::env::var("DEER_KNOCK").is_ok() {
        app.add_systems(
            Update,
            debug_knock.run_if(in_state(crate::AppState::Playing)),
        );
    }
    app.add_observer(on_knock).add_systems(
        PostUpdate,
        (start_flop, flop)
            .chain()
            .after(bevy::app::AnimationSystems)
            .before(TransformSystems::Propagate),
    );
}

/// `DEER_KNOCK=t`: knocks the player's deer flying forwards at `t` seconds, as if a wolf had
/// leapt at it from behind.
fn debug_knock(
    mut commands: Commands,
    time: Res<Time>,
    deer: Query<&LocalDeer>,
    mut done: Local<bool>,
) {
    let at = crate::debug::knob("DEER_KNOCK").unwrap_or(6.0);
    if *done || time.elapsed_secs() < at {
        return;
    }
    if let Ok(d) = deer.single() {
        *done = true;
        commands.trigger(Knock {
            dir: Vec2::new(-d.yaw.sin(), -d.yaw.cos()),
        });
    }
}

/// How high the body's middle may fly above the ground here and still stay (with the camera)
/// under the leaves:
/// from the bottoms of the crowns nearby, ignoring the lowest few (pines' skirts).
fn ceiling(w: &World, shapes: Option<&TreeShapes>, p: Vec3) -> f32 {
    let ground = w.height(p.x, p.z);
    let mut bottoms = Vec::new();
    if let Some(s) = shapes {
        w.trees_near(wrap(p.x), wrap(p.z), 25.0, |t| {
            if let Some(Some(shape)) = s.0.get(t.id as usize)
                && shape.crown_radii.y > 0.0
            {
                // The radii are 80% of the crown's box (`vegetation::place_static`).
                bottoms.push(shape.crown_center.y - shape.crown_radii.y / 0.8 - ground);
            }
        });
    }
    if bottoms.is_empty() {
        return 12.0;
    }
    bottoms.sort_by(|a, b| a.total_cmp(b));
    let low = bottoms[bottoms.len() * 3 / 10];
    // Room for the body and the camera, which rides a metre and a half above it.
    (low - 1.8).clamp(2.5, 40.0)
}

fn on_knock(
    ev: On<Knock>,
    mut commands: Commands,
    world: Res<GameWorld>,
    shapes: Option<Res<TreeShapes>>,
    heart: Option<ResMut<crate::hud::Heart>>,
    mut deer: Query<(Entity, &mut LocalDeer), Without<Knocked>>,
) {
    let Ok((e, mut d)) = deer.single_mut() else {
        return;
    };
    let dir = ev.dir.normalize_or(Vec2::NEG_Y);
    let dir = Vec2::from_angle(rand::random_range(-SPREAD..SPREAD)).rotate(dir);
    // Somewhere between skimming far and low and going up high.
    let style: f32 = rand::random();
    let elevation = 0.3 + style * 0.95;
    let speed = rand::random_range(LAUNCH.0..LAUNCH.1);
    let up_max = (2.0 * GRAVITY * ceiling(&world.0, shapes.as_deref(), d.pos)).sqrt();
    let up = (speed * elevation.sin()).min(up_max);
    let along = speed * elevation.cos();
    d.vel = Vec3::new(dir.x * along, up, dir.y * along);
    debug!(
        "knocked flying at {:.1} m/s, {:.1} up (up to {:.1} under the leaves)",
        d.vel.length(),
        up,
        up_max
    );
    d.grounded = false;
    d.action = Action::Knocked;
    d.action_left = 0.0;
    d.reach = None;
    d.approach = None;
    d.face = None;
    d.locked = false;
    // Head over heels the way it flies, a bit askew.
    let axis = Vec3::new(dir.y, 0.0, -dir.x);
    let wobble = Vec3::new(
        rand::random_range(-0.5..0.5),
        rand::random_range(-0.8..0.8),
        rand::random_range(-0.5..0.5),
    );
    let spin = (axis + wobble) * rand::random_range(5.0..11.0);
    if let Some(mut h) = heart {
        // Nearly emptied; never quite.
        h.0 = h.0.min(0.03);
    }
    commands.entity(e).insert(Knocked {
        rot: Quat::from_rotation_y(d.yaw),
        spin,
        phase: Phase::Flying,
        t: 0.0,
        fall: 0.0,
        yaw: d.yaw,
        since_thud: 0.0,
        from: Quat::IDENTITY,
        from_y: 0.0,
    });
    commands.trigger(Sfx::Thud(d.pos + Vec3::Y * CENTER, 1.0));
}

/// The trunk's circle at height `h` above the tree's base, between the traced slices. The
/// tracing stops where the trunk forks, but the trunk goes on up into the crown: the top slice,
/// a little thinner, stands in for it up to `top`.
fn slice_at(rings: &[Ring], h: f32, top: f32) -> Option<([f32; 2], f32)> {
    let first = rings.first()?;
    if h <= first.y {
        return Some((first.pos, first.r));
    }
    for pair in rings.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if h < b.y {
            let f = (h - a.y) / (b.y - a.y).max(1e-3);
            let pos = [
                a.pos[0] + wrap_delta(b.pos[0] - a.pos[0]) * f,
                a.pos[1] + wrap_delta(b.pos[1] - a.pos[1]) * f,
            ];
            return Some((pos, a.r + (b.r - a.r) * f));
        }
    }
    let last = rings.last()?;
    (h < top.max(last.y + 0.6)).then_some((last.pos, last.r * 0.8))
}

/// Pushes the body ball out of trunks (at its height) and boulders, bouncing the velocity off
/// them. Returns the hardest impact speed.
fn collide(w: &World, shapes: Option<&TreeShapes>, c: &mut Vec3, vel: &mut Vec3) -> f32 {
    let mut hardest = 0.0f32;
    let mut bounce = |c: &mut Vec3, vel: &mut Vec3, n: Vec3, depth: f32, e: f32| {
        *c += n * depth;
        let vn = vel.dot(n);
        if vn < 0.0 {
            let tangent = *vel - n * vn;
            *vel = tangent * 0.85 - n * vn * e;
            hardest = hardest.max(-vn);
        }
    };
    let at = [wrap(c.x), wrap(c.z)];
    let mut hits = Vec::new();
    w.trees_near(at[0], at[1], 6.0, |t| {
        // Trees stand 15 cm sunk into the ground (`vegetation::place_static`).
        let h = c.y - (t.y - 0.15);
        let slice = match shapes.and_then(|s| s.0.get(t.id as usize).copied().flatten()) {
            Some(s) => slice_at(s.rings(), h, s.crown_center.y - (t.y - 0.15)),
            None => (h < 4.0).then_some((t.pos, t.radius)),
        };
        if let Some((pos, r)) = slice {
            hits.push((pos, r));
        }
    });
    for (pos, r) in hits {
        let d = Vec2::new(wrap_delta(at[0] - pos[0]), wrap_delta(at[1] - pos[1]));
        let min = r + BODY;
        let len = d.length();
        if len < min {
            let n = d.normalize_or(Vec2::X);
            bounce(c, vel, Vec3::new(n.x, 0.0, n.y), min - len, TRUNK_BOUNCE);
        }
    }
    for b in &w.boulders {
        let d = Vec3::new(
            wrap_delta(at[0] - b.pos[0]),
            c.y - b.y,
            wrap_delta(at[1] - b.pos[1]),
        );
        let min = b.radius + BODY;
        let len = d.length();
        if len < min {
            bounce(c, vel, d.normalize_or(Vec3::Y), min - len, TRUNK_BOUNCE);
        }
    }
    hardest
}

/// Runs the knocked deer instead of the usual controls (in the player's chain, in its place).
pub fn tumble(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<GameWorld>,
    shapes: Option<Res<TreeShapes>>,
    mut focus: ResMut<Focus>,
    mut q: Query<(
        Entity,
        &mut LocalDeer,
        &mut Knocked,
        &mut DeerAnim,
        &mut Transform,
        &mut Reach,
    )>,
) {
    let dt = time.delta_secs().min(0.05);
    let w = &world.0;
    for (e, mut d, mut k, mut anim, mut t, mut reach) in &mut q {
        k.t += dt;
        k.since_thud += dt;
        reach.0 = None;
        d.action = Action::Knocked;
        d.reach = None;
        d.approach = None;
        d.locked = false;
        let ease = |rate: f32| 1.0 - (-dt * rate).exp();

        match k.phase {
            Phase::Flying => {
                let mut c = d.pos + Vec3::Y * CENTER;
                d.vel.y -= GRAVITY * dt;
                let drag = (-dt * 0.08).exp();
                d.vel.x *= drag;
                d.vel.z *= drag;
                c += d.vel * dt;
                let mut vel = d.vel;
                let hit = collide(w, shapes.as_deref(), &mut c, &mut vel);
                if hit > 2.5 {
                    // Knocked into a new tumble.
                    k.spin = k.spin * 0.4
                        + Vec3::new(
                            rand::random_range(-1.0..1.0),
                            rand::random_range(-1.0..1.0),
                            rand::random_range(-1.0..1.0),
                        ) * hit
                            * 0.6;
                    if k.since_thud > 0.15 {
                        commands.trigger(Sfx::Thud(c, (hit / 12.0).min(1.0)));
                        k.since_thud = 0.0;
                    }
                }
                // The ground, or the water's surface where it is too deep to stand.
                let terrain = w.height(c.x, c.z);
                let deep = terrain < WATER_LEVEL - WADE_DEPTH;
                let floor = terrain.max(WATER_LEVEL - WADE_DEPTH) + BODY * 0.8;
                let mut on_ground = false;
                if c.y < floor {
                    c.y = floor;
                    if vel.y < -3.0 {
                        let impact = -vel.y;
                        vel.y = impact * GROUND_BOUNCE;
                        vel.x *= 0.65;
                        vel.z *= 0.65;
                        k.spin *= 0.6;
                        if k.since_thud > 0.15 {
                            let p = c - Vec3::Y * BODY;
                            commands.trigger(if terrain < WATER_LEVEL {
                                Sfx::Splash(p)
                            } else {
                                Sfx::Thud(p, (impact / 15.0).min(1.0))
                            });
                            k.since_thud = 0.0;
                        }
                    } else {
                        vel.y = 0.0;
                        on_ground = true;
                        // Sliding to a stop.
                        let flat = Vec2::new(vel.x, vel.z);
                        let slowed = (flat.length() - 9.0 * dt).max(0.0);
                        let flat = flat.normalize_or_zero() * slowed;
                        vel.x = flat.x;
                        vel.z = flat.y;
                        k.spin *= (-dt * 6.0).exp();
                    }
                }
                // On the ground it rolls over onto its side, back or belly, whichever is
                // nearest: never left balanced on its nose or rump.
                if c.y <= floor + 0.05 {
                    let long = k.rot * Vec3::Z;
                    let flat = Vec3::new(long.x, 0.0, long.z);
                    if flat.length() > 1e-3 {
                        let axis = long.cross(flat.normalize());
                        let tip = Quat::from_scaled_axis(
                            axis.normalize_or_zero() * axis.length().asin().min(4.0 * dt),
                        );
                        k.rot = (tip * k.rot).normalize();
                    }
                }
                // Out of its depth: it paddles uphill, which is towards the shore.
                if on_ground && deep {
                    let n = w.normal(c.x, c.z);
                    let shore = Vec2::new(-n[0], -n[2]).normalize_or(Vec2::X) * 2.5;
                    vel.x = shore.x;
                    vel.z = shore.y;
                }
                k.spin *= (-dt * 0.3).exp();
                k.rot = (Quat::from_scaled_axis(k.spin * dt) * k.rot).normalize();
                k.fall += (AIR_FALL - k.fall) * ease(6.0);
                d.vel = vel;
                d.pos = c - Vec3::Y * CENTER;
                if on_ground && !deep && Vec2::new(vel.x, vel.z).length() < 0.8 {
                    // Lies however it landed.
                    k.phase = Phase::Lying;
                    k.t = 0.0;
                    d.vel = Vec3::ZERO;
                }
            }
            Phase::Lying => {
                if k.t > LIE_SECS {
                    k.phase = Phase::GettingUp;
                    k.t = 0.0;
                    k.from = k.rot;
                    k.from_y = d.pos.y;
                    let fwd = k.rot * Vec3::NEG_Z;
                    let flat = Vec2::new(fwd.x, fwd.z);
                    k.yaw = if flat.length() > 0.1 {
                        (-flat.x).atan2(-flat.y)
                    } else {
                        d.yaw
                    };
                }
            }
            Phase::GettingUp => {
                // A quick scramble upright, with a little hop.
                let ground = w.height(d.pos.x, d.pos.z).max(WATER_LEVEL - WADE_DEPTH);
                let f = (k.t / GET_UP_SECS).min(1.0);
                let s = f * f * (3.0 - 2.0 * f);
                k.rot = k.from.slerp(Quat::from_rotation_y(k.yaw), s).normalize();
                d.pos.y =
                    k.from_y + (ground - k.from_y) * s + (f * std::f32::consts::PI).sin() * POP_HOP;
                k.fall = AIR_FALL * (1.0 - s);
                if k.t >= GET_UP_SECS {
                    d.pos.y = ground;
                    d.yaw = k.yaw;
                    d.vel = Vec3::ZERO;
                    d.grounded = true;
                    d.action = Action::None;
                    anim.action = Action::None;
                    anim.knock = None;
                    t.translation = d.pos;
                    t.rotation = Quat::from_rotation_y(d.yaw);
                    focus.0 = d.pos;
                    commands.entity(e).remove::<Knocked>();
                    continue;
                }
            }
        }

        t.rotation = k.rot;
        t.translation = d.pos + Vec3::Y * CENTER - k.rot * (Vec3::Y * CENTER);
        anim.action = Action::Knocked;
        anim.knock = Some(k.fall);
        anim.speed = 0.0;
        anim.head_yaw = 0.0;
        anim.head_pitch = 0.0;
        focus.0 = d.pos;
    }
}

/// One floppy joint: how far it is swung off the animated pose (a rotation in the deer root's
/// space, as a scaled axis) and how fast that is changing.
struct Joint {
    bone: Entity,
    /// Ancestors up to (not including) the deer root, root-most first.
    up: Vec<Entity>,
    swing: Vec3,
    swing_vel: Vec3,
    /// The bone's own rotation under the swing, and what was last written over it. Some bones
    /// (the ears, the first tail bone) no clip animates: nothing resets those each frame, so
    /// the swing goes on top of `base` rather than of whatever is there.
    base: Quat,
    written: Quat,
    /// Spring back to the pose, pull of gravity, and the furthest it can swing (radians).
    stiff: f32,
    hang: f32,
    reach: f32,
}

/// The floppy joints of a deer's model, found when it is first knocked over.
#[derive(Component)]
pub struct Floppy {
    joints: Vec<Joint>,
    /// Per leg: the leg's end and the IK bone its hoof is skinned to (which hangs off the
    /// armature, not the leg, so it is moved to follow), each with its ancestors.
    feet: Vec<[(Entity, Vec<Entity>); 2]>,
    last_rot: Quat,
    last_pos: Vec3,
    last_vel: Vec3,
    /// The body's smoothed acceleration (world space).
    accel: Vec3,
    active: bool,
}

/// Joint names, root-most first in each chain, with (stiffness, hang, reach).
const JOINTS: [(&str, f32, f32, f32); 18] = [
    ("Torso", 70.0, 8.0, 0.35),
    ("Torso2", 70.0, 8.0, 0.35),
    ("Neck1", 28.0, 16.0, 0.8),
    ("Neck2", 28.0, 16.0, 0.8),
    ("Neck3", 28.0, 16.0, 0.8),
    ("Head", 30.0, 12.0, 0.7),
    ("FrontUpperLeg.L", 18.0, 22.0, 1.3),
    ("FrontLowerLeg.L", 24.0, 14.0, 1.1),
    ("FrontUpperLeg.R", 18.0, 22.0, 1.3),
    ("FrontLowerLeg.R", 24.0, 14.0, 1.1),
    ("BackUpperLeg.L", 18.0, 22.0, 1.2),
    ("BackLowerLeg.L", 24.0, 14.0, 1.1),
    ("BackUpperLeg.R", 18.0, 22.0, 1.2),
    ("BackLowerLeg.R", 24.0, 14.0, 1.1),
    ("Tail1", 14.0, 10.0, 1.0),
    ("Tail2", 14.0, 10.0, 1.0),
    ("Ear1.L", 30.0, 10.0, 0.9),
    ("Ear1.R", 30.0, 10.0, 0.9),
];
const FEET: [(&str, &str); 4] = [
    ("FrontLowerLeg.L_end", "IKFrontLeg.L"),
    ("FrontLowerLeg.R_end", "IKFrontLeg.R"),
    ("BackLowerLeg.L_end", "IKBackLeg.L"),
    ("BackLowerLeg.R_end", "IKBackLeg.R"),
];

/// Finds the floppy joints of any deer knocked over for the first time.
fn start_flop(
    mut commands: Commands,
    deer: Query<(Entity, &DeerAnim, &Transform), (With<crate::deer::Deer>, Without<Floppy>)>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
) {
    for (root, anim, t) in &deer {
        if anim.knock.is_none() {
            continue;
        }
        let find = |name: &str| {
            children
                .iter_descendants(root)
                .find(|e| names.get(*e).is_ok_and(|n| n.as_str() == name))
        };
        let ancestors = |mut e: Entity| {
            let mut out = Vec::new();
            while let Ok(&ChildOf(p)) = parents.get(e) {
                if p == root {
                    break;
                }
                out.push(p);
                e = p;
            }
            out.reverse();
            out
        };
        let joints: Vec<Joint> = JOINTS
            .iter()
            .filter_map(|&(name, stiff, hang, reach)| {
                let bone = find(name)?;
                Some(Joint {
                    bone,
                    up: ancestors(bone),
                    swing: Vec3::ZERO,
                    swing_vel: Vec3::ZERO,
                    base: Quat::IDENTITY,
                    written: Quat::NAN,
                    stiff,
                    hang,
                    reach,
                })
            })
            .collect();
        if joints.is_empty() {
            // The model has not spawned yet.
            continue;
        }
        let feet = FEET
            .iter()
            .filter_map(|&(end, ik)| {
                let (end, ik) = (find(end)?, find(ik)?);
                Some([(end, ancestors(end)), (ik, ancestors(ik))])
            })
            .collect();
        commands.entity(root).insert(Floppy {
            joints,
            feet,
            last_rot: t.rotation,
            last_pos: t.translation,
            last_vel: Vec3::ZERO,
            accel: Vec3::ZERO,
            active: false,
        });
    }
}

/// A bone's transform in the deer root's space; `up` its ancestors up to the root.
fn root_space(
    bone: Entity,
    up: &[Entity],
    transforms: &Query<&mut Transform>,
) -> bevy::math::Affine3A {
    up.iter()
        .chain([&bone])
        .filter_map(|&a| transforms.get(a).ok())
        .fold(bevy::math::Affine3A::IDENTITY, |p, t| {
            p * t.compute_affine()
        })
}

/// Swings each floppy joint on top of the animated pose.
fn flop(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut deer: Query<(Entity, &DeerAnim, &mut Floppy)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs().min(0.05);
    if dt <= 0.0 {
        return;
    }
    for (root, anim, mut fl) in &mut deer {
        let Ok(&rt) = transforms.get(root) else {
            continue;
        };
        // Every joint back to its own pose before swinging it again.
        for j in &mut fl.joints {
            if let Ok(mut t) = transforms.get_mut(j.bone) {
                if t.rotation == j.written {
                    t.rotation = j.base;
                } else {
                    j.base = t.rotation;
                }
            }
        }
        let Some(fall) = anim.knock else {
            fl.active = false;
            fl.last_rot = rt.rotation;
            fl.last_pos = rt.translation;
            continue;
        };
        let fl = &mut *fl;
        // The body's middle, and how it moved and turned since last frame.
        let mid = rt.translation + rt.rotation * Vec3::Y * CENTER;
        let last_mid = fl.last_pos + fl.last_rot * Vec3::Y * CENTER;
        let step = mid - last_mid;
        let vel = if fl.active && step.length() < 5.0 {
            step / dt
        } else {
            fl.last_vel
        };
        if !fl.active {
            fl.active = true;
            fl.last_vel = vel;
            fl.accel = Vec3::ZERO;
            for j in &mut fl.joints {
                j.swing = Vec3::ZERO;
                j.swing_vel = Vec3::ZERO;
            }
        }
        let raw = ((vel - fl.last_vel) / dt).clamp_length_max(120.0);
        fl.accel += (raw - fl.accel) * (1.0 - (-dt * 12.0).exp());
        fl.last_vel = vel;
        let turn = (rt.rotation * fl.last_rot.inverse()).to_scaled_axis();
        let turn = if turn.length() < 1.0 {
            turn
        } else {
            Vec3::ZERO
        };
        fl.last_rot = rt.rotation;
        fl.last_pos = rt.translation;

        let inv = rt.rotation.inverse();
        // Gravity as the body feels it (less its own acceleration), in its own space, so a
        // free-falling body floats its limbs and a hard bounce flings them.
        let airborne = mid.y - world.0.height(mid.x, mid.z) > BODY + 0.2;
        let felt = inv * (Vec3::NEG_Y * GRAVITY - fl.accel) / GRAVITY;
        let felt = if airborne { felt } else { felt * 0.25 };
        // Limp while down; stiffening back into the pose as it gets up.
        let limp = (fall / AIR_FALL).clamp(0.0, 1.0);
        let firm = 1.0 + (1.0 - limp) * 6.0;
        let spin_back = inv * turn;

        // Where each leg ends and how it is turned, before the swings.
        let ends_before: Vec<(Vec3, Quat)> = fl
            .feet
            .iter()
            .map(|[(e, up), _]| {
                let (_, rot, pos) = root_space(*e, up, &transforms).to_scale_rotation_translation();
                (pos, rot)
            })
            .collect();
        for j in &mut fl.joints {
            let parent =
                j.up.iter()
                    .filter_map(|&a| transforms.get(a).ok())
                    .fold(Quat::IDENTITY, |p, t| p * t.rotation);
            let Ok(mut t) = transforms.get_mut(j.bone) else {
                continue;
            };
            // Bones run along their +Y.
            let axis = parent * Quat::from_scaled_axis(j.swing) * t.rotation * Vec3::Y;
            let stiff = j.stiff * firm;
            let damp = 2.0 * stiff.sqrt() * 0.3;
            let pull = axis.cross(felt) * j.hang * limp;
            j.swing_vel += (pull - j.swing * stiff - j.swing_vel * damp) * dt;
            // Left behind as the body turns under it.
            j.swing += j.swing_vel * dt - spin_back * 0.6 * limp;
            if j.swing.length() > j.reach {
                j.swing = j.swing.clamp_length_max(j.reach);
                j.swing_vel *= 0.5;
            }
            let delta = Quat::from_scaled_axis(j.swing);
            let local = (parent.inverse() * delta * parent * t.rotation).normalize();
            if local.is_finite() {
                t.rotation = local;
            }
            j.written = t.rotation;
        }
        // The hooves go where the legs now end, turned as the legs turned (or the skin between
        // shin and hoof would stretch and twist).
        for (i, [(end, end_up), (foot, foot_up)]) in fl.feet.iter().enumerate() {
            let (_, rot, pos) =
                root_space(*end, end_up, &transforms).to_scale_rotation_translation();
            let (pos_before, rot_before) = ends_before[i];
            let parent = foot_up
                .iter()
                .filter_map(|&a| transforms.get(a).ok())
                .fold(bevy::math::Affine3A::IDENTITY, |p, t| {
                    p * t.compute_affine()
                });
            let (_, parent_rot, _) = parent.to_scale_rotation_translation();
            let turned = rot * rot_before.inverse();
            if let Ok(mut t) = transforms.get_mut(*foot) {
                t.translation += parent.matrix3.inverse() * (pos - pos_before);
                // The hoof turns about the leg's end, which is where it is.
                let local = (parent_rot.inverse() * turned * parent_rot * t.rotation).normalize();
                if local.is_finite() {
                    t.rotation = local;
                }
            }
        }
    }
}
