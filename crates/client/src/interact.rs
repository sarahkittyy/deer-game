//! Eating and drinking. The middle of the screen picks a berry (generously: a wide sphere around
//! each one) or the water, when the deer is close enough; the berry glows and the cursor turns
//! into a berry or a glass of water.
//!
//! A click on a berry starts a bite the player cannot interrupt: the deer walks up and turns to
//! the berry, reaches until its mouth is on it, and only then the berry goes and the munch
//! plays; it chews, and can eat again after a short pause. A click on the water lowers the head
//! all the way to the surface for a few sips (walking off ends it).

use crate::{
    AppState, GameWorld,
    audio::Sfx,
    bushes::{Berry, BerryLook, Bush, Bushes},
    deer::Mouth,
    env::MainCamera,
    net::Net,
    player::{LocalDeer, cursor_locked},
    ripples::RippleAt,
    ui::{Crosshair, CursorIcon},
};
use bevy::{
    prelude::*,
    window::{CursorOptions, PrimaryWindow},
};
use deer_shared::{
    protocol::{Action, ClientMsg},
    world::WATER_LEVEL,
};

/// Generous radius around a berry that counts as pointing at it.
const BERRY_PICK_RADIUS: f32 = 0.32;
/// How far from the deer's mouth a berry or the water can be reached.
const REACH: f32 = 2.4;
const WATER_REACH: f32 = 3.6;
const DRINK_SECS: f32 = 5.0;
/// Longest the deer may walk towards the water before it drinks from where it stands.
const WADE_IN_SECS: f32 = 1.5;
/// How far in front of the body the mouth meets a berry or the water, horizontally.
const MOUTH_AHEAD: f32 = 0.95;
/// Longest the deer may take to get its mouth onto a berry before it bites anyway.
const APPROACH_SECS: f32 = 1.4;
const CHEW_SECS: f32 = 0.9;
/// Pause after chewing before the next bite.
const COOLDOWN_SECS: f32 = 0.5;
/// The mouth counts as on the berry / in the water within this distance.
const TOUCH: f32 = 0.12;
const SIP_EVERY: f32 = 1.1;
/// How far in front of the body the mouth meets the water: past the front hooves.
const DRINK_AHEAD: f32 = 1.1;

/// A bite in progress.
#[derive(Resource)]
struct Bite {
    berry: Entity,
    slot: usize,
    index: u8,
    generation: u32,
    /// Where the berry hangs (render space).
    at: Vec3,
    t: f32,
    bitten: bool,
}

/// Sips while drinking.
#[derive(Resource, Default)]
struct Sips {
    next: f32,
}

/// Seconds until the next bite may start.
#[derive(Resource, Default)]
struct Cooldown(f32);

#[derive(Resource, Default, PartialEq)]
enum Target {
    #[default]
    None,
    Berry(Entity),
    /// The water, at this point (render space).
    Water(Vec3),
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Target>()
        .init_resource::<Sips>()
        .init_resource::<Cooldown>()
        .add_systems(
            Update,
            (
                pick,
                highlight,
                act,
                bite.run_if(resource_exists::<Bite>),
                drink,
            )
                .chain()
                .after(crate::player::PlayerSet)
                .run_if(in_state(AppState::Playing)),
        );
}

fn ray_sphere(o: Vec3, d: Vec3, c: Vec3, r: f32) -> Option<f32> {
    let oc = o - c;
    let b = oc.dot(d);
    let disc = b * b - (oc.length_squared() - r * r);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

fn pick(
    cam: Single<&GlobalTransform, With<MainCamera>>,
    deer: Query<&LocalDeer>,
    berries: Query<(Entity, &GlobalTransform), With<Berry>>,
    world: Res<GameWorld>,
    mut target: ResMut<Target>,
) {
    let Ok(d) = deer.single() else { return };
    let o = cam.translation();
    let dir = cam.forward().as_vec3();
    let mouth = d.head();

    let mut best: Option<(f32, Entity)> = None;
    for (e, t) in &berries {
        let c = t.translation();
        if c.distance(mouth) > REACH {
            continue;
        }
        if let Some(hit) = ray_sphere(o, dir, c, BERRY_PICK_RADIUS)
            && best.is_none_or(|(b, _)| hit < b)
        {
            best = Some((hit, e));
        }
    }
    let new = if let Some((_, e)) = best {
        Target::Berry(e)
    } else if dir.y < -0.05 {
        let t = (WATER_LEVEL - o.y) / dir.y;
        let p = o + dir * t;
        let near = Vec2::new(p.x - mouth.x, p.z - mouth.z).length() < WATER_REACH;
        if near && world.0.height(p.x, p.z) < WATER_LEVEL - 0.02 {
            Target::Water(p)
        } else {
            Target::None
        }
    } else {
        Target::None
    };
    if *target != new {
        *target = new;
    }
}

fn highlight(
    target: Res<Target>,
    look: Option<Res<BerryLook>>,
    mut berries: Query<(Entity, &mut MeshMaterial3d<StandardMaterial>), With<Berry>>,
    mut cross: Query<&mut Visibility, (With<Crosshair>, Without<CursorIcon>)>,
    mut icons: Query<(&CursorIcon, &mut Visibility), Without<Crosshair>>,
    mut last: Local<Option<Entity>>,
) {
    if !target.is_changed() {
        return;
    }
    let now = match *target {
        Target::Berry(e) => Some(e),
        _ => None,
    };
    // (A berry can only be targeted once the berry materials exist.)
    if let Some(look) = look
        && *last != now
    {
        for e in [*last, now].into_iter().flatten() {
            if let Ok((e, mut m)) = berries.get_mut(e) {
                m.0 = if Some(e) == now {
                    look.glow.clone()
                } else {
                    look.normal.clone()
                };
            }
        }
        *last = now;
    }
    // The dot, or a berry / a glass of water in its place.
    let show = match *target {
        Target::Berry(_) => Some(CursorIcon::Berry),
        Target::Water(_) => Some(CursorIcon::Water),
        Target::None => None,
    };
    if let Ok(mut v) = cross.single_mut() {
        *v = if show.is_none() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (icon, mut v) in &mut icons {
        *v = if Some(*icon) == show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

#[allow(clippy::too_many_arguments)]
fn act(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    target: Res<Target>,
    berries: Query<(&Berry, &GlobalTransform, &ChildOf)>,
    bush_q: Query<&Bush>,
    biting: Option<Res<Bite>>,
    mut cooldown: ResMut<Cooldown>,
    mut sips: ResMut<Sips>,
    world: Res<GameWorld>,
    mut deer: Query<&mut LocalDeer>,
    mut was_locked: Local<bool>,
) {
    cooldown.0 -= time.delta_secs();
    // The click that captures the mouse is not also a bite.
    let locked = cursor_locked(&cursor);
    let click = mouse.just_pressed(MouseButton::Left) && *was_locked;
    *was_locked = locked;
    if !click || biting.is_some() || cooldown.0 > 0.0 {
        return;
    }
    let Ok(mut d) = deer.single_mut() else {
        return;
    };
    if !d.grounded {
        return;
    }
    match *target {
        Target::Berry(e) => {
            let Ok((berry, t, parent)) = berries.get(e) else {
                return;
            };
            let Ok(bush) = bush_q.get(parent.0) else {
                return;
            };
            commands.insert_resource(Bite {
                berry: e,
                slot: berry.slot,
                index: berry.index,
                generation: bush.generation,
                at: t.translation(),
                t: 0.0,
                bitten: false,
            });
            d.locked = true;
            d.action = Action::Eating;
            d.reach = Some(t.translation());
        }
        Target::Water(o) => {
            // Face the spot that was clicked and walk towards it until the mouth is over the
            // water (but not out of wading depth), then lower the head to the surface.
            let here = Vec2::new(d.pos.x, d.pos.z);
            let to = Vec2::new(o.x, o.z) - here;
            if to.length() > 0.05 {
                let dir = to.normalize();
                let face = (-dir.x).atan2(-dir.y);
                let wet = |p: Vec2| world.0.height(p.x, p.y) < WATER_LEVEL - 0.02;
                let mut spot = here;
                let mut along = 0.0;
                while along < to.length() {
                    let p = here + dir * along;
                    if world.0.too_deep(p.x, p.y) {
                        break;
                    }
                    spot = p;
                    if wet(p + dir * DRINK_AHEAD) {
                        break;
                    }
                    along += 0.1;
                }
                d.face = Some(face);
                if spot.distance(here) > 0.05 {
                    d.approach = Some((spot, face));
                }
            }
            d.start(Action::Drinking, DRINK_SECS);
            sips.next = 0.0;
        }
        Target::None => {}
    }
}

/// The bite: walk up, reach, bite, chew, done.
#[allow(clippy::too_many_arguments)]
fn bite(
    mut commands: Commands,
    time: Res<Time>,
    mut bite: ResMut<Bite>,
    berries: Query<(), With<Berry>>,
    mut bushes: ResMut<Bushes>,
    mut net: ResMut<Net>,
    mut cooldown: ResMut<Cooldown>,
    mut deer: Query<(&mut LocalDeer, &Mouth)>,
) {
    let Ok((mut d, mouth)) = deer.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    bite.t += dt;
    let finish = |d: &mut LocalDeer, commands: &mut Commands, cooldown: &mut Cooldown| {
        d.locked = false;
        d.approach = None;
        d.reach = None;
        d.action = Action::None;
        cooldown.0 = COOLDOWN_SECS;
        commands.remove_resource::<Bite>();
    };
    if !bite.bitten {
        // Someone else got there first.
        if berries.get(bite.berry).is_err() {
            finish(&mut d, &mut commands, &mut cooldown);
            return;
        }
        // Stand where the mouth meets the berry, facing it.
        let here = Vec2::new(d.pos.x, d.pos.z);
        let berry = Vec2::new(bite.at.x, bite.at.z);
        let away = (here - berry).normalize_or(Vec2::Y);
        let spot = berry + away * MOUTH_AHEAD;
        let face = (-(berry.x - spot.x)).atan2(-(berry.y - spot.y));
        d.approach = Some((spot, face));
        d.reach = Some(bite.at);
        let close = mouth.mouth.distance(bite.at) < TOUCH;
        if close || bite.t > APPROACH_SECS {
            // The mouth is on it: now it goes.
            bite.bitten = true;
            bite.t = 0.0;
            bushes.eat(bite.slot, bite.index);
            net.send(&ClientMsg::Eat {
                slot: bite.slot as u16,
                generation: bite.generation,
                berry: bite.index,
            });
            commands.trigger(Sfx::Eat(bite.at));
        }
    } else {
        // Chew: the head stays put, bobbing a little.
        d.approach = None;
        let bob = (bite.t * 13.0).sin() * 0.035;
        d.reach = Some(bite.at + Vec3::Y * bob);
        if bite.t > CHEW_SECS {
            finish(&mut d, &mut commands, &mut cooldown);
        }
    }
}

/// While drinking: the mouth goes down to the water's surface in front of the deer, and sips.
fn drink(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<GameWorld>,
    mut sips: ResMut<Sips>,
    mut deer: Query<(&mut LocalDeer, &Mouth)>,
) {
    let Ok((mut d, mouth)) = deer.single_mut() else {
        return;
    };
    if d.action != Action::Drinking {
        return;
    }
    // Stop walking in once there, or once it has had long enough (something in the way).
    if let Some((spot, _)) = d.approach
        && (spot.distance(Vec2::new(d.pos.x, d.pos.z)) < 0.06
            || DRINK_SECS - d.action_left > WADE_IN_SECS)
    {
        d.approach = None;
    }
    let ahead = d.pos + Quat::from_rotation_y(d.yaw) * Vec3::new(0.0, 0.0, -DRINK_AHEAD);
    let surface = world.0.height(ahead.x, ahead.z).max(WATER_LEVEL) + 0.02;
    let at = Vec3::new(ahead.x, surface, ahead.z);
    d.reach = Some(at);
    if mouth.mouth.distance(at) < TOUCH * 1.6 {
        sips.next -= time.delta_secs();
        if sips.next <= 0.0 {
            sips.next = SIP_EVERY;
            commands.trigger(Sfx::Drink(mouth.mouth));
            if surface <= WATER_LEVEL + 0.03 {
                commands.trigger(RippleAt {
                    at: Vec3::new(mouth.mouth.x, WATER_LEVEL, mouth.mouth.z),
                    size: 0.7,
                });
            }
        }
    }
}
