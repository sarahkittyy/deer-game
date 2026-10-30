//! Everyone else's deer: drawn from the server's snapshots a little in the past, interpolated
//! between them, with a name tag floating above each.

use crate::{
    AppState, GameEntity,
    deer::{self, DeerAnim, DeerAssets},
    env::MainCamera,
    net::{Net, NetEvent},
    ui::UiFont,
    wrap::Wrapped,
};
use bevy::prelude::*;
use deer_shared::{
    protocol::{Action, Pose},
    world::{wrap, wrap_delta},
};
use std::collections::{HashMap, VecDeque};

/// How far behind the newest snapshot ghosts are drawn: a bit over one snapshot interval, so
/// there is nearly always a pair to interpolate between.
const DELAY: f64 = 0.12;
/// Name tags fade out between these distances.
const TAG_NEAR: f32 = 25.0;
const TAG_FAR: f32 = 45.0;

#[derive(Component)]
pub struct Ghost {
    pub id: u32,
    snapshots: VecDeque<(f64, Pose)>,
    pub tag: Entity,
}

#[derive(Component)]
pub struct NameTag;

/// Names of the players the server told us about, by id.
#[derive(Resource, Default)]
pub struct Roster(HashMap<u32, (String, bool)>);

impl Roster {
    pub fn name(&self, id: u32) -> Option<String> {
        self.0.get(&id).map(|(name, _)| name.clone())
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Roster>()
        .add_systems(
            Update,
            (on_net, interpolate, place_tags)
                .chain()
                // After the camera has moved this frame, so the tags do not trail it.
                .after(crate::player::PlayerSet)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(OnExit(AppState::Playing), |mut r: ResMut<Roster>| {
            r.0.clear()
        });
}

fn on_net(
    mut commands: Commands,
    mut events: MessageReader<NetEvent>,
    time: Res<Time>,
    net: Res<Net>,
    assets: Res<DeerAssets>,
    font: Res<UiFont>,
    mut roster: ResMut<Roster>,
    mut ghosts: Query<(Entity, &mut Ghost)>,
) {
    let now = time.elapsed_secs_f64();
    // Ghosts spawned this frame are not in the query yet; their later snapshots wait a tick.
    let mut spawned = std::collections::HashSet::new();
    for ev in events.read() {
        match ev {
            NetEvent::Joined { id, name, stag } => {
                roster.0.insert(*id, (name.clone(), *stag));
            }
            NetEvent::Left { id } => {
                roster.0.remove(id);
                for (e, g) in &ghosts {
                    if g.id == *id {
                        commands.entity(g.tag).despawn();
                        commands.entity(e).despawn();
                    }
                }
            }
            NetEvent::Disconnected | NetEvent::Welcome { .. } => {
                for (e, g) in &ghosts {
                    commands.entity(g.tag).despawn();
                    commands.entity(e).despawn();
                }
                if matches!(ev, NetEvent::Disconnected) {
                    roster.0.clear();
                }
            }
            NetEvent::Snapshot { poses } => {
                for (id, pose) in poses {
                    if Some(*id) == net.my_id {
                        continue;
                    }
                    if let Some((_, mut g)) = ghosts.iter_mut().find(|(_, g)| g.id == *id) {
                        g.snapshots.push_back((now, *pose));
                        while g.snapshots.len() > 12 {
                            g.snapshots.pop_front();
                        }
                        continue;
                    }
                    let Some((name, stag)) = roster.0.get(id) else {
                        continue;
                    };
                    if !spawned.insert(*id) {
                        continue;
                    }
                    let tag = commands
                        .spawn((
                            NameTag,
                            GameEntity,
                            Text::new(name.clone()),
                            TextFont {
                                font: font.0.clone().into(),
                                font_size: FontSize::Px(14.0),
                                ..default()
                            },
                            TextColor(Color::srgba(1.0, 0.97, 0.9, 0.0)),
                            TextShadow {
                                offset: Vec2::new(1.0, 1.0),
                                color: Color::srgba(0.0, 0.0, 0.0, 0.0),
                            },
                            Node {
                                position_type: PositionType::Absolute,
                                ..default()
                            },
                            Visibility::Hidden,
                        ))
                        .id();
                    let e = deer::spawn(&mut commands, &assets, *stag);
                    let p = Vec3::from_array(pose.pos);
                    commands.entity(e).insert((
                        GameEntity,
                        Wrapped(p),
                        Transform::from_translation(p),
                        Ghost {
                            id: *id,
                            snapshots: VecDeque::from([(now, *pose)]),
                            tag,
                        },
                    ));
                }
            }
            _ => {}
        }
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    a + d * t
}

fn interpolate(
    time: Res<Time>,
    mut ghosts: Query<(&Ghost, &mut Wrapped, &mut Transform, &mut DeerAnim)>,
) {
    let render = time.elapsed_secs_f64() - DELAY;
    for (g, mut w, mut t, mut anim) in &mut ghosts {
        let s = &g.snapshots;
        let Some(&(_, last)) = s.back() else { continue };
        // The pair around the render time, or the newest pose when we have run past it.
        let (a, b, f) = match s.iter().position(|(ts, _)| *ts > render) {
            Some(0) => (s[0].1, s[0].1, 0.0),
            Some(i) => {
                let (t0, p0) = s[i - 1];
                let (t1, p1) = s[i];
                (p0, p1, ((render - t0) / (t1 - t0).max(1e-3)) as f32)
            }
            None => (last, last, 0.0),
        };
        let pos = Vec3::new(
            wrap(a.pos[0] + wrap_delta(b.pos[0] - a.pos[0]) * f),
            a.pos[1] + (b.pos[1] - a.pos[1]) * f,
            wrap(a.pos[2] + wrap_delta(b.pos[2] - a.pos[2]) * f),
        );
        w.0 = pos;
        t.rotation = Quat::from_rotation_y(lerp_angle(a.body_yaw, b.body_yaw, f));
        // Knocked flying: the whole body turned about its middle, and the legs limp.
        let tumble = match (a.tumble, b.tumble) {
            (Some(ta), Some(tb)) => Some((
                Quat::from_array(ta.rot).slerp(Quat::from_array(tb.rot), f),
                ta.fall + (tb.fall - ta.fall) * f,
            )),
            (Some(k), None) | (None, Some(k)) => Some((Quat::from_array(k.rot), k.fall)),
            (None, None) => None,
        };
        anim.knock = tumble.map(|(_, fall)| fall);
        if let Some((rot, _)) = tumble {
            let c = Vec3::Y * crate::knock::CENTER;
            let p = pos + c - rot.normalize() * c;
            w.0 = Vec3::new(wrap(p.x), p.y, wrap(p.z));
            t.rotation = rot.normalize();
        }
        anim.speed = a.speed + (b.speed - a.speed) * f;
        anim.pace = b.pace;
        anim.head_yaw = a.head_yaw + (b.head_yaw - a.head_yaw) * f;
        anim.head_pitch = a.head_pitch + (b.head_pitch - a.head_pitch) * f;
        anim.action = if f < 0.5 { a.action } else { b.action };
        if anim.action == Action::Airborne && b.action != Action::Airborne {
            anim.action = b.action;
        }
    }
}

/// Keeps each name tag over its deer's head, fading with distance and hidden behind the camera.
/// Everything is taken from this frame's values:
/// the camera's `Transform` (it has no parent, so it is its global transform) as the player
/// code just set it, and each ghost's wrapped position placed near the player's, rather than
/// the `GlobalTransform`s, which are only updated after `Update` and would be a frame old.
pub fn place_tags(
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    ui_size: Single<&ComputedNode, With<crate::ui::UiRoot>>,
    focus: Res<crate::wrap::Focus>,
    ghosts: Query<(&Ghost, &Wrapped, &Transform)>,
    mut tags: Query<
        (
            &mut Node,
            &mut Visibility,
            &mut TextColor,
            &mut TextShadow,
            &ComputedNode,
        ),
        With<NameTag>,
    >,
) {
    let (cam, cam_t) = *camera;
    let cam_t = GlobalTransform::from(*cam_t);
    let cam_t = &cam_t;
    for (g, w, t) in &ghosts {
        let Ok((mut node, mut vis, mut color, mut shadow, computed)) = tags.get_mut(g.tag) else {
            continue;
        };
        // Just above the head, which is forward of the body's centre.
        let at = crate::wrap::nearest(w.0, focus.0);
        let head = at + t.rotation * Vec3::new(0.0, 1.95, -0.55);
        let dist = head.distance(cam_t.translation());
        let alpha = 1.0 - ((dist - TAG_NEAR) / (TAG_FAR - TAG_NEAR)).clamp(0.0, 1.0);
        // Through normalised coordinates: the 3D view may render at a lower resolution than
        // the UI it is shown under (`render_scale`).
        let screen_size = ui_size.size() * ui_size.inverse_scale_factor();
        let on_screen = cam
            .world_to_ndc(cam_t, head)
            .filter(|n| n.z > 0.0 && n.z < 1.0)
            .map(|n| Vec2::new((n.x + 1.0) * 0.5, (1.0 - n.y) * 0.5) * screen_size);
        match on_screen {
            Some(screen) if alpha > 0.0 => {
                let size = computed.size() * computed.inverse_scale_factor();
                node.left = Val::Px(screen.x - size.x * 0.5);
                node.top = Val::Px(screen.y - size.y);
                *vis = Visibility::Inherited;
                color.0.set_alpha(alpha);
                shadow.color.set_alpha(alpha * 0.8);
            }
            _ => *vis = Visibility::Hidden,
        }
    }
}
