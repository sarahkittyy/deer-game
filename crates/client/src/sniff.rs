//! Sniffing out company: stand still for a moment and the deer lifts its nose twice, then a
//! little antlers marker on a ring around the crosshair points towards the nearest deer that is
//! out of sight, for a few seconds (less if that deer comes into sight). Deer already in sight
//! are ignored, and with nobody out there the deer does not sniff at all. One sniff per stop:
//! the deer has to move before it sniffs again.

use crate::{
    AppState, GameEntity,
    audio::Sfx,
    ghosts::Ghost,
    player::{LocalDeer, Look, PlayerSet},
    wrap::{Focus, Wrapped},
};
use bevy::prelude::*;
use deer_shared::protocol::Action;

/// Standing still this long starts a sniff.
const STILL_SECS: f32 = 1.5;
/// Deer nearer than this count as in sight: not sniffed out, and the marker's goal.
const SIGHT: f32 = 25.0;
/// The two upward flicks of the head.
const SNIFF_SECS: f32 = 0.8;
const FLICK: f32 = 0.55;
/// The head also turns this far to one side and back while sniffing (radians).
const TURN: f32 = 0.3;
/// The sniff sound starts this far in, so its two breaths land on the two flicks.
const SOUND_AT: f32 = 0.1;
/// How long the marker stays before fading.
const SHOW_SECS: f32 = 3.0;
/// The marker's ring around the crosshair, and how fast it fades.
const RING: f32 = 64.0;
const FADE_PER_SEC: f32 = 2.0;
const MARK_SIZE: f32 = 26.0;
const ARROW: f32 = 7.0;
const CREAM: Color = Color::srgb(0.93, 0.87, 0.76);

#[derive(Resource, Default)]
struct Sniff {
    /// How long the deer has stood still.
    still: f32,
    /// Already sniffed during this stop.
    done: bool,
    /// Time into the head flicks, while they play.
    flicking: Option<f32>,
    /// Which way the head turns during this sniff: -1 or 1.
    side: f32,
    /// How long the marker has been up.
    showing: f32,
    /// The ghost the marker points at.
    target: Option<u32>,
    shown: f32,
}

#[derive(Component)]
struct Marker;

#[derive(Component)]
struct MarkerArrow;

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Playing), spawn)
        .add_systems(
            Update,
            (sniff, show)
                .chain()
                .after(PlayerSet)
                .run_if(in_state(AppState::Playing)),
        );
}

fn spawn(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Sniff::default());
    commands.spawn((
        Marker,
        GameEntity,
        Node {
            position_type: PositionType::Absolute,
            width: px(MARK_SIZE),
            height: px(MARK_SIZE),
            ..default()
        },
        ImageNode::new(assets.load("ui/antlers.png")).with_color(CREAM.with_alpha(0.0)),
        Pickable::IGNORE,
    ));
    // The arrow: a square turned on its corner, its outer half showing past a clip.
    let arrow = commands
        .spawn((
            MarkerArrow,
            GameEntity,
            Node {
                position_type: PositionType::Absolute,
                width: px(ARROW * 2.0),
                height: px(ARROW),
                overflow: Overflow::clip(),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    let side = ARROW * std::f32::consts::SQRT_2;
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: px(side),
            height: px(side),
            // Centred on the clip's bottom edge: the top half shows, a triangle pointing up.
            top: px(ARROW - side * 0.5),
            ..default()
        },
        UiTransform::from_rotation(Rot2::degrees(45.0)),
        BackgroundColor(CREAM),
        ChildOf(arrow),
    ));
}

/// Where each other deer is, relative to ours (horizontal, the short way round the world).
fn others<'a>(
    me: Vec3,
    focus: Vec3,
    ghosts: impl Iterator<Item = (&'a Ghost, &'a Wrapped)>,
) -> impl Iterator<Item = (u32, Vec2)> {
    ghosts.map(move |(g, w)| {
        let p = crate::wrap::nearest(w.0, focus);
        (g.id, Vec2::new(p.x - me.x, p.z - me.z))
    })
}

fn sniff(
    time: Res<Time>,
    focus: Res<Focus>,
    sniff: Option<ResMut<Sniff>>,
    mut commands: Commands,
    mut deer: Query<(&LocalDeer, &mut crate::deer::DeerAnim, &crate::deer::Mouth)>,
    ghosts: Query<(&Ghost, &Wrapped)>,
) {
    let (Some(mut sniff), Ok((d, mut anim, mouth))) = (sniff, deer.single_mut()) else {
        return;
    };
    let dt = time.delta_secs();
    let still = anim.speed < 0.15 && d.grounded && d.action == Action::None;
    if still {
        sniff.still += dt;
    } else {
        sniff.still = 0.0;
        sniff.done = false;
    }
    if sniff.still >= STILL_SECS && !sniff.done {
        sniff.done = true;
        // The nearest deer out of sight; none out there, no sniff.
        let far = others(d.pos, focus.0, ghosts.iter())
            .filter(|(_, v)| v.length() > SIGHT)
            .min_by(|a, b| a.1.length().total_cmp(&b.1.length()));
        if let Some((id, _)) = far {
            sniff.target = Some(id);
            sniff.flicking = Some(0.0);
            sniff.side = if rand::random::<bool>() { 1.0 } else { -1.0 };
            sniff.showing = 0.0;
        }
    }
    // Two quick lifts of the nose with the head turned a little aside, on top of wherever the
    // head was looking, and a soft sniff.
    if let Some(before) = sniff.flicking {
        let t = before + dt;
        if before < SOUND_AT && t >= SOUND_AT {
            commands.trigger(Sfx::Sniff(mouth.mouth));
        }
        if t >= SNIFF_SECS {
            sniff.flicking = None;
        } else {
            sniff.flicking = Some(t);
            let half = SNIFF_SECS * 0.5;
            let bump = (std::f32::consts::PI * (t % half) / half).sin().powi(2);
            anim.head_pitch += FLICK * bump;
            anim.head_yaw += TURN * sniff.side * (std::f32::consts::PI * t / SNIFF_SECS).sin();
        }
    }
}

#[allow(clippy::type_complexity)]
fn show(
    time: Res<Time>,
    focus: Res<Focus>,
    look: Res<Look>,
    sniff: Option<ResMut<Sniff>>,
    deer: Query<&LocalDeer>,
    ghosts: Query<(&Ghost, &Wrapped)>,
    mut marker: Query<(&mut Node, &mut ImageNode), (With<Marker>, Without<MarkerArrow>)>,
    mut arrow: Query<
        (&mut Node, &mut UiTransform, &mut Visibility, &Children),
        (With<MarkerArrow>, Without<Marker>),
    >,
    mut tip: Query<&mut BackgroundColor>,
    ui_size: Single<&ComputedNode, With<crate::ui::UiRoot>>,
) {
    let (Some(mut sniff), Ok(d)) = (sniff, deer.single()) else {
        return;
    };
    // Where the deer being sniffed out is, if it is still around and out of sight.
    let goal = sniff.target.and_then(|id| {
        others(d.pos, focus.0, ghosts.iter())
            .find(|(g, _)| *g == id)
            .map(|(_, v)| v)
    });
    if sniff.flicking.is_none() && sniff.target.is_some() {
        sniff.showing += time.delta_secs();
    }
    let wanted = goal.is_some_and(|v| v.length() > SIGHT)
        && sniff.flicking.is_none()
        && sniff.showing < SHOW_SECS;
    let step = FADE_PER_SEC * time.delta_secs();
    sniff.shown = if wanted {
        (sniff.shown + step).min(1.0)
    } else {
        (sniff.shown - step).max(0.0)
    };
    if sniff.shown <= 0.0 && sniff.flicking.is_none() && !wanted {
        sniff.target = None;
    }
    let (Ok((mut mnode, mut image)), Ok((mut anode, mut turn, mut avis, kids))) =
        (marker.single_mut(), arrow.single_mut())
    else {
        return;
    };
    image.color.set_alpha(sniff.shown * 0.9);
    let Some(v) = goal.filter(|_| sniff.shown > 0.0) else {
        *avis = Visibility::Hidden;
        return;
    };
    // On the ring: up is straight ahead of the camera, right is to its right.
    let fwd = Vec2::new(-look.yaw.sin(), -look.yaw.cos());
    let right = Vec2::new(-fwd.y, fwd.x);
    let dir = Vec2::new(v.dot(right), -v.dot(fwd)).normalize_or(Vec2::NEG_Y);
    let centre = ui_size.size() * ui_size.inverse_scale_factor() * 0.5;
    let at = centre + dir * RING;
    mnode.left = px(at.x - MARK_SIZE * 0.5);
    mnode.top = px(at.y - MARK_SIZE * 0.5);
    // The arrow just outside the marker, pointing on outwards.
    let tip_at = centre + dir * (RING + MARK_SIZE * 0.5 + ARROW);
    anode.left = px(tip_at.x - ARROW);
    anode.top = px(tip_at.y - ARROW * 0.5);
    turn.rotation = Rot2::radians(dir.x.atan2(-dir.y));
    *avis = Visibility::Inherited;
    for k in kids {
        if let Ok(mut bg) = tip.get_mut(*k) {
            bg.0 = CREAM.with_alpha(sniff.shown * 0.9);
        }
    }
}
