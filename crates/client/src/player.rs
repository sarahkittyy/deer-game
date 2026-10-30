//! The player's own deer: WASD relative to the camera, shift to gallop (forward only), space to
//! jump, the body turning gradually towards where it moves, and collisions with the ground,
//! trunks, boulders and the water's edge. Also the over-the-shoulder camera.

use crate::{
    AppState, GameWorld,
    audio::Sfx,
    deer::{self, DeerAnim, DeerAssets, RUN_SPEED, Reach, WALK_SPEED},
    env::MainCamera,
    vegetation::TreeShapes,
    wrap::Focus,
};
use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use deer_shared::{
    protocol::{Action, Pose},
    world::{WADE_DEPTH, WATER_LEVEL, WORLD_SIZE, World, wrap, wrapped_dist2},
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

const GRAVITY: f32 = 16.0;
const JUMP_SPEED: f32 = 5.6;
const ACCEL: f32 = 7.0;
const DECEL: f32 = 9.0;
/// Turn rate of the body towards the movement direction (rad/s); slower at a gallop.
const TURN_WALK: f32 = 4.2;
const TURN_RUN: f32 = 2.4;
/// How much slower the deer goes at the deepest it can wade (a fraction of its speed).
const WADE_DRAG: f32 = 0.25;
/// The deer's collision circle and how far ahead its nose reaches (for the water's edge).
const BODY_RADIUS: f32 = 0.35;
const NOSE: f32 = 0.75;
/// Trunk slices higher than this pass over the deer.
const DEER_HEIGHT: f32 = 1.7;
/// Head-look limits relative to the body.
const HEAD_YAW_MAX: f32 = 1.1;
const HEAD_PITCH_UP: f32 = 0.5;
const HEAD_PITCH_DOWN: f32 = 0.7;
const MOUSE_SENSITIVITY: f32 = 0.0025;

/// The local player's deer.
#[derive(Component)]
pub struct LocalDeer {
    /// Unwrapped position of the feet.
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub grounded: bool,
    /// Eating/drinking (or the jump) and how long it has left.
    pub action: Action,
    pub action_left: f32,
    /// Smoothed slope tilt of the body.
    tilt: f32,
    /// Distance walked since the last footstep sound.
    stride: f32,
    /// Where the mouth should reach (render space), while eating or drinking.
    pub reach: Option<Vec3>,
    /// While set, the deer walks itself to this spot (unwrapped x, z) and turns to this yaw:
    /// the lead-up to a bite or a drink.
    pub approach: Option<(Vec2, f32)>,
    /// Turn to face this yaw (drinking), keys still allowed.
    pub face: Option<f32>,
    /// While true, the keys do nothing (a bite in progress).
    pub locked: bool,
}

impl LocalDeer {
    pub fn pose(&self, anim: &DeerAnim) -> Pose {
        Pose {
            pos: [wrap(self.pos.x), self.pos.y, wrap(self.pos.z)],
            body_yaw: self.yaw,
            head_yaw: anim.head_yaw,
            head_pitch: anim.head_pitch,
            speed: anim.speed,
            pace: anim.pace,
            action: anim.action,
            // Filled in by `net::send_pose`, which knows the camera and any tumble.
            look: self.yaw,
            tumble: None,
        }
    }

    /// Where the eyes are, roughly: for reach checks and the camera.
    pub fn head(&self) -> Vec3 {
        self.pos + Vec3::Y * 1.3 + Quat::from_rotation_y(self.yaw) * Vec3::new(0.0, 0.0, -0.55)
    }

    /// Starts eating or drinking for `secs`.
    pub fn start(&mut self, action: Action, secs: f32) {
        self.action = action;
        self.action_left = secs;
    }
}

/// The player's movement and camera systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerSet;

/// Camera orbit angles driven by the mouse.
#[derive(Resource, Default)]
pub struct Look {
    pub yaw: f32,
    pub pitch: f32,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Look>()
        .add_systems(
            Update,
            (
                grab_cursor,
                mouse_look,
                movement,
                crate::knock::tumble,
                follow_camera,
            )
                .chain()
                .in_set(PlayerSet)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(OnExit(AppState::Playing), release_cursor);
}

/// Spawns the player's deer standing at wrapped position (x, z).
pub fn spawn_local(
    commands: &mut Commands,
    assets: &DeerAssets,
    world: &World,
    stag: bool,
    at: [f32; 2],
    look: &mut Look,
) -> Entity {
    let y = world.height(at[0], at[1]);
    let yaw = crate::debug::yaw_override().unwrap_or_else(|| {
        deer_shared::noise::unit(deer_shared::noise::hash64(at[0].to_bits() as u64)) * TAU
    });
    look.yaw = yaw;
    look.pitch = -0.15;
    let e = deer::spawn(commands, assets, stag);
    commands.entity(e).insert((
        LocalDeer {
            pos: Vec3::new(at[0], y, at[1]),
            vel: Vec3::ZERO,
            yaw,
            grounded: true,
            action: Action::None,
            action_left: 0.0,
            tilt: 0.0,
            stride: 0.0,
            reach: None,
            approach: None,
            face: None,
            locked: false,
        },
        Reach::default(),
        crate::GameEntity,
    ));
    e
}

fn grab_cursor(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<crate::chat::ChatOpen>,
    window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let (window, mut cursor) = window.into_inner();
    if crate::debug::scripted_clicks_on() {
        return;
    }
    // Tabbed away (Cmd-Tab, clicking another app): let the mouse go. A click back in the
    // window captures it again.
    if !window.focused {
        if cursor.grab_mode != CursorGrabMode::None {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
        return;
    }
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    // Esc with the chat box open only closes the box.
    if keys.just_pressed(KeyCode::Escape) && !chat.0 {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn release_cursor(mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

pub fn cursor_locked(cursor: &CursorOptions) -> bool {
    // Scripted test clicks act as if the mouse were captured, without taking the real one.
    cursor.grab_mode != CursorGrabMode::None || crate::debug::scripted_clicks_on()
}

fn mouse_look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut look: ResMut<Look>,
) {
    if !cursor_locked(&cursor) {
        return;
    }
    let d = motion.delta * MOUSE_SENSITIVITY;
    look.yaw = (look.yaw - d.x).rem_euclid(TAU);
    look.pitch = (look.pitch - d.y).clamp(-1.2, 0.9);
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + PI).rem_euclid(TAU) - PI
}

fn movement(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    world: Res<GameWorld>,
    shapes: Option<Res<TreeShapes>>,
    look: Res<Look>,
    chat: Res<crate::chat::ChatOpen>,
    heart: Option<Res<crate::hud::Heart>>,
    mut focus: ResMut<Focus>,
    // A knocked-over deer is run by `knock::tumble` instead.
    mut q: Query<
        (&mut LocalDeer, &mut DeerAnim, &mut Transform, &mut Reach),
        Without<crate::knock::Knocked>,
    >,
) {
    let dt = time.delta_secs().min(0.05);
    let w = &world.0;
    for (mut d, mut anim, mut t, mut reach) in &mut q {
        // Typing a chat line, or held still for a bite: the keys do not walk.
        let key = |k| !d.locked && !chat.0 && keys.pressed(k);
        let f = key(KeyCode::KeyW) as i32 as f32 - key(KeyCode::KeyS) as i32 as f32;
        let r = key(KeyCode::KeyD) as i32 as f32 - key(KeyCode::KeyA) as i32 as f32;
        // Sprint is forward only: any strafe or backing up cancels it.
        let sprint = key(KeyCode::ShiftLeft) && f > 0.0 && r == 0.0;
        let input = Vec2::new(r, f);
        let moving = input != Vec2::ZERO;

        // Walking off ends a drink (a bite cannot be interrupted: the keys are locked).
        d.action_left -= dt;
        if d.action == Action::Drinking && (moving || d.action_left <= 0.0) {
            d.action = Action::None;
        }
        // Whatever ended a drink (walking off, time, a jump), its pull on the body goes with it.
        // A bite keeps the keys locked until it lets go itself.
        if d.action != Action::Drinking && !d.locked {
            d.reach = None;
            d.face = None;
            d.approach = None;
        }

        // Camera-relative direction on the ground. Camera yaw 0 looks along -Z.
        let fwd = Vec3::new(-look.yaw.sin(), 0.0, -look.yaw.cos());
        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
        let mut wish = if moving {
            (fwd * input.y + right * input.x).normalize()
        } else {
            Vec3::ZERO
        };
        // A full heart makes for a livelier deer: half again as fast; an empty one, half as.
        let pace = 0.5 + heart.as_ref().map_or(0.5, |h| h.0);
        anim.pace = pace;
        let mut speed = if sprint { RUN_SPEED } else { WALK_SPEED } * pace;
        // Walking itself up to a berry: towards the spot, slowing as it arrives.
        if let Some((spot, _)) = d.approach {
            let to = spot - Vec2::new(d.pos.x, d.pos.z);
            let dist = to.length();
            wish = if dist > 0.03 {
                Vec3::new(to.x, 0.0, to.y) / dist
            } else {
                Vec3::ZERO
            };
            // A steady walk that slows over the last few steps, not a glide that creeps in.
            speed = (dist * 2.5).clamp(0.45, WALK_SPEED * 0.8);
        }
        let moving = moving || wish != Vec3::ZERO;
        // Water drags a little: up to a quarter slower at the deepest the deer can wade.
        let depth = (WATER_LEVEL - w.height(d.pos.x, d.pos.z)).clamp(0.0, WADE_DEPTH);
        speed *= 1.0 - WADE_DRAG * depth / WADE_DEPTH;
        let target = wish * speed;
        let horiz = Vec3::new(d.vel.x, 0.0, d.vel.z);
        let rate = if moving { ACCEL } else { DECEL };
        // Less control in the air.
        let rate = if d.grounded { rate } else { rate * 0.25 };
        let horiz = horiz + (target - horiz) * (1.0 - (-dt * rate).exp());
        d.vel.x = horiz.x;
        d.vel.z = horiz.z;

        // Turn the body gradually towards where it is heading (or what it is about to eat).
        let hspeed = horiz.length();
        let yaw_before = d.yaw;
        if let Some(face) = d.approach.map(|a| a.1).or(d.face) {
            let max = 5.0 * dt;
            let diff = angle_diff(face, d.yaw);
            d.yaw = (d.yaw + diff.clamp(-max, max)).rem_euclid(TAU);
        } else if hspeed > 0.15 {
            let want = (-horiz.x).atan2(-horiz.z);
            let max = if hspeed > WALK_SPEED * 1.3 {
                TURN_RUN
            } else {
                TURN_WALK
            } * dt;
            let diff = angle_diff(want, d.yaw);
            d.yaw = (d.yaw + diff.clamp(-max, max)).rem_euclid(TAU);
        }

        // Turning on the spot moves the feet too (they are about half a metre from the middle).
        let turning = if dt > 0.0 {
            angle_diff(d.yaw, yaw_before).abs() / dt * 0.5
        } else {
            0.0
        };

        if !d.locked && !chat.0 && keys.just_pressed(KeyCode::Space) && d.grounded {
            d.vel.y = JUMP_SPEED;
            d.grounded = false;
            d.action = Action::Airborne;
            d.action_left = 0.0;
        }

        // Horizontal move with collisions, one axis at a time so the deer slides along things.
        let step = Vec3::new(d.vel.x, 0.0, d.vel.z) * dt;
        let mut p = d.pos;
        let heading = Quat::from_rotation_y(d.yaw) * Vec3::NEG_Z;
        for axis in [Vec3::X, Vec3::Z] {
            let candidate = p + step * axis;
            // Past the wall already (turning on the spot can swing the nose over deep water):
            // any step that is no deeper is allowed, so the deer can always walk back out.
            let over = too_deep_by(w, candidate, heading);
            if over <= 0.0 || over < too_deep_by(w, p, heading) {
                p = candidate;
            } else if axis == Vec3::X {
                d.vel.x = 0.0;
            } else {
                d.vel.z = 0.0;
            }
        }
        p = push_out_of_obstacles(w, shapes.as_deref(), p);

        // Vertical: gravity, landing, and sticking to the ground when walking downhill.
        let ground = w.height(p.x, p.z);
        d.vel.y -= GRAVITY * dt;
        p.y += d.vel.y * dt;
        if p.y <= ground || (d.grounded && p.y - ground < 0.35 && d.vel.y <= 0.0) {
            p.y = ground;
            d.vel.y = 0.0;
            if !d.grounded && d.action == Action::Airborne {
                d.action = Action::None;
                commands.trigger(Sfx::Land(p));
            }
            d.grounded = true;
        } else {
            d.grounded = false;
        }

        // Keep the unwrapped position near the origin without changing what is seen.
        let span = WORLD_SIZE * 8.0;
        if p.x.abs() > span || p.z.abs() > span {
            p.x = wrap(p.x);
            p.z = wrap(p.z);
        }
        if d.grounded {
            d.stride += Vec2::new(p.x - d.pos.x, p.z - d.pos.z).length();
            // Four hooves: a step sound every so often, further apart at a gallop.
            let every = if hspeed > WALK_SPEED * 1.5 { 1.4 } else { 0.55 };
            if d.stride > every {
                d.stride = 0.0;
                commands.trigger(if w.height(p.x, p.z) < WATER_LEVEL - 0.02 {
                    Sfx::Splash(p)
                } else {
                    Sfx::Step(p)
                });
            }
        }
        d.pos = p;

        // Lean the body with the slope it stands on.
        let ahead = d.pos + heading * 0.6;
        let behind = d.pos - heading * 0.6;
        let slope = (w.height(ahead.x, ahead.z) - w.height(behind.x, behind.z)).atan2(1.2);
        let want_tilt = if d.grounded {
            slope.clamp(-0.4, 0.4)
        } else {
            d.tilt
        };
        d.tilt += (want_tilt - d.tilt) * (1.0 - (-dt * 8.0).exp());

        t.translation = d.pos;
        t.rotation = Quat::from_rotation_y(d.yaw) * Quat::from_rotation_x(d.tilt);

        // The head follows the camera, within limits, unless it is reaching for something
        // (then `deer::reach` steers it).
        reach.0 = d.reach;
        if d.reach.is_none() {
            let rel_yaw = angle_diff(look.yaw, d.yaw);
            anim.head_yaw = rel_yaw.clamp(-HEAD_YAW_MAX, HEAD_YAW_MAX);
            anim.head_pitch = (look.pitch + 0.15).clamp(-HEAD_PITCH_DOWN, HEAD_PITCH_UP);
            // Looking far behind: the head gives up and comes back to the middle.
            if rel_yaw.abs() > FRAC_PI_2 + 0.6 {
                anim.head_yaw = 0.0;
            }
        }
        anim.speed = hspeed.max(turning);
        anim.action = d.action;
        focus.0 = d.pos;
    }
}

/// How much deeper than the deer can wade the water is under its body or its nose (metres;
/// zero or less where it can stand).
fn too_deep_by(w: &World, p: Vec3, heading: Vec3) -> f32 {
    let nose = p + heading * NOSE;
    let over = |x: f32, z: f32| WATER_LEVEL - WADE_DEPTH - w.height(x, z);
    over(p.x, p.z).max(over(nose.x, nose.z))
}

/// Pushes the body circle out of trunks and boulders.
fn push_out_of_obstacles(w: &World, shapes: Option<&TreeShapes>, mut p: Vec3) -> Vec3 {
    let at = [wrap(p.x), wrap(p.z)];
    let mut push = Vec2::ZERO;
    let mut resolve = |center: [f32; 2], radius: f32| {
        let min = radius + BODY_RADIUS;
        if wrapped_dist2(center, at) < min * min {
            let dx = deer_shared::world::wrap_delta(at[0] - center[0]);
            let dz = deer_shared::world::wrap_delta(at[1] - center[1]);
            let d = Vec2::new(dx, dz);
            let len = d.length().max(1e-4);
            push += d / len * (min - len);
        }
    };
    w.trees_near(at[0], at[1], 5.0, |t| {
        match shapes.and_then(|s| s.0.get(t.id as usize).copied().flatten()) {
            // Every slice of the trunk the deer's body could touch, so a leaning trunk is
            // solid where it leans, not just where it meets the ground.
            Some(s) => {
                for ring in s.rings().iter().filter(|r| r.y < DEER_HEIGHT) {
                    resolve(ring.pos, ring.r);
                }
            }
            None => resolve(t.pos, t.radius),
        }
    });
    for b in &w.boulders {
        resolve(b.pos, b.radius);
    }
    p.x += push.x;
    p.z += push.y;
    p
}

/// Over the right shoulder, a few metres back, kept above the ground.
fn follow_camera(
    time: Res<Time>,
    world: Res<GameWorld>,
    shapes: Option<Res<TreeShapes>>,
    look: Res<Look>,
    deer: Query<&LocalDeer>,
    mut cam: Single<&mut Transform, With<MainCamera>>,
    mut smoothed: Local<Option<Vec3>>,
    mut pulled: Local<f32>,
) {
    let Ok(d) = deer.single() else { return };
    let rot = Quat::from_rotation_y(look.yaw) * Quat::from_rotation_x(look.pitch);
    // Smooth the pivot a little so jumps and bumps do not jolt the view.
    let pivot_target = d.pos + Vec3::Y * 1.55;
    let rate = 1.0 - (-time.delta_secs() * 14.0).exp();
    let pivot = match *smoothed {
        Some(p) if p.distance(pivot_target) < 3.0 => p + (pivot_target - p) * rate,
        _ => pivot_target,
    };
    *smoothed = Some(pivot);
    let mut eye = pivot + rot * Vec3::new(0.55, 0.25, 2.9);
    // Trunks between the deer and the camera: pull the camera in, in front of them. It
    // snaps in (to never show the inside of a trunk) and eases back out.
    let mut reach = 1.0f32;
    let ray = eye - pivot;
    let flat = Vec2::new(ray.x, ray.z);
    if flat.length_squared() > 1e-4 {
        let o = Vec2::new(wrap(pivot.x), wrap(pivot.z));
        let ground = world.0.height(pivot.x, pivot.z);
        world.0.trees_near(o.x, o.y, 9.0, |t| {
            let shape = shapes
                .as_ref()
                .and_then(|s| s.0.get(t.id as usize).copied().flatten());
            let fallback = [crate::vegetation::Ring {
                y: 0.0,
                pos: t.pos,
                r: t.radius,
            }];
            let rings = match &shape {
                Some(s) => s.rings(),
                None => &fallback[..],
            };
            for ring in rings {
                // Only slices near the height the camera passes at.
                let along = |k: f32| pivot.y + ray.y * k - ground;
                if ring.y > along(0.0).max(along(1.0)) + 1.0 {
                    continue;
                }
                let r = ring.r + 0.35;
                let c = Vec2::new(
                    deer_shared::world::wrap_delta(ring.pos[0] - o.x),
                    deer_shared::world::wrap_delta(ring.pos[1] - o.y),
                );
                // Nearest hit of the segment pivot -> eye (t in 0..1) with the slice's circle.
                let a = flat.dot(flat);
                let b = -2.0 * flat.dot(c);
                let cc = c.dot(c) - r * r;
                let disc = b * b - 4.0 * a * cc;
                if disc >= 0.0 {
                    let hit = (-b - disc.sqrt()) / (2.0 * a);
                    if (0.0..1.0).contains(&hit) {
                        reach = reach.min(hit);
                    }
                }
            }
        });
    }
    let reach = reach.max(0.12);
    *pulled = if reach < *pulled {
        reach
    } else {
        *pulled + (reach - *pulled) * (1.0 - (-time.delta_secs() * 4.0).exp())
    };
    eye = pivot + ray * *pulled;
    let floor = world.0.height(eye.x, eye.z) + 0.35;
    if eye.y < floor {
        eye.y = floor;
    }
    **cam = Transform::from_translation(eye).with_rotation(rot);
}
