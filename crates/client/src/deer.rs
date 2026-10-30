//! The deer model, shared by the player's own deer and everyone else's: loading the doe and stag,
//! blending their animations from speed and action, and turning the neck and head towards where
//! the deer is looking.

use bevy::{
    animation::RepeatAnimation, app::AnimationSystems, math::Affine3A, prelude::*,
    world_serialization::WorldInstanceReady,
};
use deer_shared::protocol::Action;

/// Quaternius models are ~4.3 units tall; this makes a deer about 1.4 m at the head.
const MODEL_SCALE: f32 = 0.32;
/// Speeds (m/s) the walk and gallop clips are authored for; playback is scaled from these.
const WALK_CLIP_SPEED: f32 = 1.6;
const GALLOP_CLIP_SPEED: f32 = 7.0;
pub const WALK_SPEED: f32 = 1.9;
pub const RUN_SPEED: f32 = 7.5;

/// Clip indices in both glTF files (the clips without the `AnimalArmature|` prefix).
mod clip {
    pub const EATING: usize = 3;
    pub const GALLOP: usize = 4;
    pub const GALLOP_JUMP: usize = 5;
    pub const JUMP_TO_IDLE: usize = 8;
    pub const WALK: usize = 9;
    pub const IDLE_HEADLOW: usize = 10;
    pub const IDLE: usize = 12;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Clip {
    Idle,
    Walk,
    Gallop,
    Jump,
    StandJump,
    Eat,
    HeadLow,
}
const CLIPS: [(Clip, usize); 7] = [
    (Clip::Idle, clip::IDLE),
    (Clip::Walk, clip::WALK),
    (Clip::Gallop, clip::GALLOP),
    (Clip::Jump, clip::GALLOP_JUMP),
    (Clip::StandJump, clip::JUMP_TO_IDLE),
    (Clip::Eat, clip::EATING),
    (Clip::HeadLow, clip::IDLE_HEADLOW),
];

struct ModelAssets {
    scene: Handle<WorldAsset>,
    graph: Handle<AnimationGraph>,
    nodes: [AnimationNodeIndex; 7],
}

#[derive(Resource)]
pub struct DeerAssets {
    doe: ModelAssets,
    stag: ModelAssets,
}

/// What drives a deer's animation. The player's controller or the network fills it in.
#[derive(Component, Default, Clone, Copy)]
pub struct DeerAnim {
    pub speed: f32,
    pub action: Action,
    /// Where the head looks, relative to the body: yaw left/right, pitch up/down (radians).
    pub head_yaw: f32,
    pub head_pitch: f32,
}

/// Where a deer's neck starts and where its mouth is, as drawn (render space, last frame), and
/// the head turn that was applied to get there.
#[derive(Component, Default, Clone, Copy)]
pub struct Mouth {
    pub neck: Vec3,
    pub mouth: Vec3,
    pub look: Vec2,
}

/// Steers the head so that the mouth reaches this point (render space): drinking from the
/// water, nibbling a berry. The local deer's controller sets it; ghosts get the head angles
/// that result over the network.
#[derive(Component, Default)]
pub struct Reach(pub Option<Vec3>);

/// Root of a deer. Its children hold the model, scaled and turned to face -Z.
#[derive(Component)]
pub struct Deer {
    pub stag: bool,
}

/// Blend state, kept on the deer root once its model has spawned.
#[derive(Component)]
struct DeerRig {
    player: Entity,
    nodes: [AnimationNodeIndex; 7],
    weights: [f32; 7],
    /// Bones that share the head turn: the three neck bones and the head, root-most first.
    neck: [Entity; 4],
    /// Ancestors of the first neck bone up to (not including) the deer root, root-most first.
    neck_ancestors: Vec<Entity>,
    last_action: Action,
    /// Smoothed head turn actually applied.
    look: Vec2,
    /// Bones that lower the front end to drink: the chest, then the left and right front legs,
    /// each with its ancestors up to the deer root (root-most first).
    crouch_bones: [(Entity, Vec<Entity>); 3],
    /// Per front leg: the leg's end and the foot bone the hoof is skinned to, each with its
    /// ancestors. The foot hangs off the armature, not the leg, so it is moved to follow.
    feet: [[(Entity, Vec<Entity>); 2]; 2],
    /// How far into the drinking crouch, 0 to 1.
    crouch: f32,
}

pub fn plugin(app: &mut App) {
    app.add_systems(PreStartup, load)
        .add_systems(Update, (reach, blend).chain())
        .add_systems(
            PostUpdate,
            (
                turn_head
                    .after(AnimationSystems)
                    .before(TransformSystems::Propagate),
                measure_mouth.after(TransformSystems::Propagate),
            ),
        );
}

fn load_model(
    path: &'static str,
    assets: &AssetServer,
    graphs: &mut Assets<AnimationGraph>,
) -> ModelAssets {
    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let nodes = CLIPS.map(|(_, i)| {
        // Graph weight 1: the per-frame mix is set on the player's active animations.
        graph.add_clip(
            assets.load(GltfAssetLabel::Animation(i).from_asset(path)),
            1.0,
            root,
        )
    });
    ModelAssets {
        scene: assets.load(GltfAssetLabel::Scene(0).from_asset(path)),
        graph: graphs.add(graph),
        nodes,
    }
}

fn load(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    commands.insert_resource(DeerAssets {
        doe: load_model("models/doe.glb", &assets, &mut graphs),
        stag: load_model("models/stag.glb", &assets, &mut graphs),
    });
}

/// Spawns a deer root (add `Transform` and anything else to the returned entity).
pub fn spawn(commands: &mut Commands, assets: &DeerAssets, stag: bool) -> Entity {
    let model = if stag { &assets.stag } else { &assets.doe };
    let root = commands
        .spawn((
            Deer { stag },
            DeerAnim::default(),
            Mouth::default(),
            Transform::default(),
            Visibility::default(),
        ))
        .id();
    commands
        .spawn((
            WorldAssetRoot(model.scene.clone()),
            Transform::from_scale(Vec3::splat(MODEL_SCALE))
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            ChildOf(root),
        ))
        .observe(on_model_ready);
    root
}

fn on_model_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    assets: Res<DeerAssets>,
    parents: Query<&ChildOf>,
    deers: Query<&Deer>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut players: Query<&mut AnimationPlayer>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let model_root = ready.entity;
    let Ok(&ChildOf(deer)) = parents.get(model_root) else {
        return;
    };
    let Ok(d) = deers.get(deer) else { return };
    let model = if d.stag { &assets.stag } else { &assets.doe };

    let mut player_entity = None;
    let mut bones: [Option<Entity>; 4] = [None; 4];
    let mut crouch: [Option<Entity>; 3] = [None; 3];
    let mut feet: [Option<Entity>; 4] = [None; 4];

    for e in children.iter_descendants(model_root) {
        // The models come from FBX with 40% metal, which mirrors the sky; fur is matte.
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
                "Neck1" => bones[0] = Some(e),
                "Neck2" => bones[1] = Some(e),
                "Neck3" => bones[2] = Some(e),
                "Head" => bones[3] = Some(e),
                "Torso2" => crouch[0] = Some(e),
                "FrontUpperLeg.L" => crouch[1] = Some(e),
                "FrontUpperLeg.R" => crouch[2] = Some(e),
                "FrontLowerLeg.L_end" => feet[0] = Some(e),
                "IKFrontLeg.L" => feet[1] = Some(e),
                "FrontLowerLeg.R_end" => feet[2] = Some(e),
                "IKFrontLeg.R" => feet[3] = Some(e),
                _ => {}
            }
        }
    }
    let (
        Some(player_entity),
        [Some(n1), Some(n2), Some(n3), Some(head)],
        [Some(torso), Some(leg_l), Some(leg_r)],
        [Some(end_l), Some(foot_l), Some(end_r), Some(foot_r)],
    ) = (player_entity, bones, crouch, feet)
    else {
        warn!("deer model is missing its animation player, neck or leg bones");
        return;
    };
    // Ancestors of a bone up to (not including) the deer root, root-most first.
    let ancestors = |mut e: Entity| {
        let mut out = Vec::new();
        while let Ok(&ChildOf(p)) = parents.get(e) {
            if p == deer {
                break;
            }
            out.push(p);
            e = p;
        }
        out.reverse();
        out
    };

    let mut player = players.get_mut(player_entity).unwrap();
    for (i, node) in model.nodes.iter().enumerate() {
        let a = player.play(*node);
        a.set_weight(1e-3);
        if matches!(CLIPS[i].0, Clip::Jump | Clip::StandJump) {
            a.set_repeat(RepeatAnimation::Never);
        } else {
            a.repeat();
        }
    }
    commands
        .entity(player_entity)
        .insert(AnimationGraphHandle(model.graph.clone()));
    commands.entity(deer).insert(DeerRig {
        player: player_entity,
        nodes: model.nodes,
        weights: [0.0; 7],
        neck: [n1, n2, n3, head],
        neck_ancestors: ancestors(n1),
        last_action: Action::None,
        look: Vec2::ZERO,
        crouch_bones: [
            (torso, ancestors(torso)),
            (leg_l, ancestors(leg_l)),
            (leg_r, ancestors(leg_r)),
        ],
        crouch: 0.0,
        feet: [
            [(end_l, ancestors(end_l)), (foot_l, ancestors(foot_l))],
            [(end_r, ancestors(end_r)), (foot_r, ancestors(foot_r))],
        ],
    });
}

fn blend(
    time: Res<Time>,
    mut rigs: Query<(&DeerAnim, &mut DeerRig)>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let dt = time.delta_secs();
    for (anim, mut rig) in &mut rigs {
        let Ok(mut player) = players.get_mut(rig.player) else {
            continue;
        };
        let rig = &mut *rig;
        let started = anim.action != rig.last_action;
        rig.last_action = anim.action;

        // Target weights: locomotion blends idle -> walk -> gallop by speed; actions override.
        let s = anim.speed;
        let mut target = [0.0f32; 7];
        let walk_t = (s / WALK_SPEED).clamp(0.0, 1.0);
        let run_t = ((s - WALK_SPEED) / (RUN_SPEED * 0.6 - WALK_SPEED)).clamp(0.0, 1.0);
        target[0] = 1.0 - walk_t;
        target[1] = walk_t * (1.0 - run_t);
        target[2] = walk_t * run_t;
        let over = match anim.action {
            Action::Airborne => Some(if s > WALK_SPEED {
                Clip::Jump
            } else {
                Clip::StandJump
            }),
            // Eating and drinking keep the walk: the deer steps itself into place first. The
            // head is steered onto the berry or the water (`reach`) on top.
            Action::Eating | Action::Drinking | Action::None => None,
        };
        if let Some(c) = over {
            let i = CLIPS.iter().position(|(k, _)| *k == c).unwrap();
            target.fill(0.0);
            target[i] = 1.0;
            if started && let Some(a) = player.animation_mut(rig.nodes[i]) {
                a.replay();
                // Skip the jump clips' crouch: the deer is already in the air.
                a.seek_to(if c == Clip::Jump { 0.25 } else { 0.0 });
            }
        }

        // Idle stays on underneath everything: some clips (eating) leave bones like Neck3
        // unanimated, and a bone no active clip covers gets no pose at all.
        target[0] = target[0].max(0.02);
        let rate = 1.0 - (-dt * 9.0).exp();
        for i in 0..7 {
            rig.weights[i] += (target[i] - rig.weights[i]) * rate;
            if let Some(a) = player.animation_mut(rig.nodes[i]) {
                // Never exactly zero: Bevy blends by dividing by the running weight total, and
                // two zero-weight clips on one bone give 0/0.
                a.set_weight(rig.weights[i].max(1e-3));
            }
        }
        // Pace the feet to the ground speed.
        if let Some(a) = player.animation_mut(rig.nodes[1]) {
            a.set_speed((s / WALK_CLIP_SPEED).clamp(0.4, 1.6));
        }
        if let Some(a) = player.animation_mut(rig.nodes[2]) {
            a.set_speed((s / GALLOP_CLIP_SPEED).clamp(0.6, 1.3));
        }
    }
}

/// Head look on top of the animation: the yaw/pitch is split across the neck and head bones.
/// Each bone's rotation is changed so that, in the deer's own space, it turns by its share
/// about the deer's up axis and then its right axis.
fn turn_head(
    time: Res<Time>,
    mut rigs: Query<(&DeerAnim, &mut DeerRig)>,
    mut transforms: Query<&mut Transform>,
) {
    // The whole neck bends, the base most: deep bends (drinking) look like a deer's.
    const SHARE: [f32; 4] = [0.34, 0.26, 0.2, 0.2];
    let rate = 1.0 - (-time.delta_secs() * 6.0).exp();
    for (anim, mut rig) in &mut rigs {
        let target = Vec2::new(anim.head_yaw, anim.head_pitch);
        let look = rig.look + (target - rig.look) * rate;
        rig.look = look;

        // Drinking: the chest tips down and the front legs spread apart (and lean back to stay
        // upright), which brings the shoulders low enough for the mouth to reach the water.
        // The hooves go where the legs now end.
        let want = if anim.action == Action::Drinking {
            1.0
        } else {
            0.0
        };
        rig.crouch += (want - rig.crouch) * (1.0 - (-time.delta_secs() * 3.0).exp());
        let c = rig.crouch;
        if c > 1e-3 {
            let [(torso, torso_up), (leg_l, leg_l_up), (leg_r, leg_r_up)] = &rig.crouch_bones;
            let ends_before = rig
                .feet
                .each_ref()
                .map(|[(e, up), _]| deer_space(*e, up, &transforms).translation);
            let tip = Quat::from_rotation_x(-CROUCH_TIP * c);
            turn_in_deer_space(*torso, torso_up, tip, &mut transforms);
            let upright = Quat::from_rotation_x(CROUCH_TIP * c);
            for (leg, up, side) in [(leg_l, leg_l_up, -1.0), (leg_r, leg_r_up, 1.0)] {
                let splay = Quat::from_rotation_z(side * CROUCH_SPLAY * c);
                turn_in_deer_space(*leg, up, splay * upright, &mut transforms);
            }
            for (i, [(end, end_up), (foot, foot_up)]) in rig.feet.iter().enumerate() {
                let moved = deer_space(*end, end_up, &transforms).translation - ends_before[i];
                let parent = foot_up
                    .iter()
                    .filter_map(|&a| transforms.get(a).ok())
                    .fold(Affine3A::IDENTITY, |p, t| p * t.compute_affine());
                if let Ok(mut t) = transforms.get_mut(*foot) {
                    t.translation += Vec3::from(parent.matrix3.inverse() * moved);
                }
            }
        }

        // Rotation of the first neck bone's parent, in the deer root's space.
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
            // Deer space: +Y up, -Z forward, so +X is the deer's right.
            let delta =
                Quat::from_rotation_y(look.x * SHARE[k]) * Quat::from_rotation_x(look.y * SHARE[k]);
            let local = (parent.inverse() * delta * parent * t.rotation).normalize();
            if local.is_finite() {
                t.rotation = local;
            }
            parent *= t.rotation;
        }

        // The muzzle (the head bone's +Y) never curls back under the chin: it stops a little
        // short of straight down, and when drinking it goes there.
        if let Ok(mut t) = transforms.get_mut(rig.neck[3]) {
            let above = parent * t.rotation.inverse();
            let dir = parent * Vec3::Y;
            let heading = Vec3::new(-look.x.sin(), 0.0, -look.x.cos());
            let pitch = dir.y.clamp(-1.0, 1.0).asin();
            let backwards = Vec3::new(dir.x, 0.0, dir.z).dot(heading) < 0.0;
            let mut want = if backwards {
                -MUZZLE_LOWEST
            } else {
                pitch.max(-MUZZLE_LOWEST)
            };
            want += (-MUZZLE_LOWEST - want) * c;
            if backwards || (want - pitch).abs() > 1e-3 {
                let to = heading * want.cos() + Vec3::Y * want.sin();
                let fix = Quat::from_rotation_arc(dir.normalize(), to);
                let local = (above.inverse() * fix * parent).normalize();
                if local.is_finite() {
                    t.rotation = local;
                }
            }
        }
    }
}

/// A bone's transform in the deer root's space; `ancestors` up to the root, root-most first.
fn deer_space(bone: Entity, ancestors: &[Entity], transforms: &Query<&mut Transform>) -> Affine3A {
    ancestors
        .iter()
        .chain([&bone])
        .filter_map(|&a| transforms.get(a).ok())
        .fold(Affine3A::IDENTITY, |p, t| p * t.compute_affine())
}

/// How far the chest tips forward and the front legs splay out when drinking (radians).
const CROUCH_TIP: f32 = 0.22;
const CROUCH_SPLAY: f32 = 0.45;
/// How far below level the muzzle may point (radians); straight down is 1.57.
const MUZZLE_LOWEST: f32 = 1.3;
/// From the head bone along the muzzle to the mouth.
const MUZZLE_LENGTH: f32 = 0.3;

/// Turns `bone` by `delta`, a rotation in the deer root's space; `ancestors` are the bone's
/// ancestors up to the deer root, root-most first.
fn turn_in_deer_space(
    bone: Entity,
    ancestors: &[Entity],
    delta: Quat,
    transforms: &mut Query<&mut Transform>,
) {
    let parent = ancestors
        .iter()
        .filter_map(|&a| transforms.get(a).ok())
        .fold(Quat::IDENTITY, |p, t| p * t.rotation);
    if let Ok(mut t) = transforms.get_mut(bone) {
        let local = (parent.inverse() * delta * parent * t.rotation).normalize();
        if local.is_finite() {
            t.rotation = local;
        }
    }
}

/// Records where the neck and mouth ended up, once the bones have their final transforms.
fn measure_mouth(mut deer: Query<(&DeerRig, &mut Mouth)>, globals: Query<&GlobalTransform>) {
    for (rig, mut m) in &mut deer {
        let (Ok(neck), Ok(head)) = (globals.get(rig.neck[0]), globals.get(rig.neck[3])) else {
            continue;
        };
        let (n, h) = (neck.translation(), head.translation());
        m.neck = n;
        m.mouth = h + head.rotation() * Vec3::Y * MUZZLE_LENGTH;
        m.look = rig.look;
    }
}

/// Steers the head onto the `Reach` point: compares where the mouth is with where it should
/// be, as angles seen from the base of the neck in the deer's own space, and turns the head by
/// the difference. A few frames bring the mouth onto the point whatever the model or pose.
fn reach(mut deer: Query<(&Reach, &Mouth, &GlobalTransform, &mut DeerAnim)>) {
    for (reach, m, root, mut anim) in &mut deer {
        let Some(target) = reach.0 else { continue };
        if m.mouth == m.neck {
            continue;
        }
        let inv = root.rotation().inverse();
        let angles = |v: Vec3| {
            let flat = Vec2::new(v.x, v.z).length();
            Vec2::new((-v.x).atan2(-v.z), v.y.atan2(flat))
        };
        let have = inv * (m.mouth - m.neck);
        let want = inv * (target - m.neck);
        let (a, b) = (angles(have), angles(want));
        let mut err = b - a;
        err.x =
            (err.x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        // Straight down, which way is "sideways" is meaningless: leave the yaw alone.
        if Vec2::new(want.x, want.z).length() < 0.15 {
            err.x = 0.0;
        }
        anim.head_yaw = (m.look.x + err.x * 0.8).clamp(-1.3, 1.3);
        anim.head_pitch = (m.look.y + err.y * 0.8).clamp(-2.0, 0.8);
    }
}
