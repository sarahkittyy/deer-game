//! Rings spreading on the water where deer wade, the player's and everyone else's, and where
//! they drink. Ghosts' steps in the water splash too (the player's own steps splash in
//! `player.rs`).

use crate::{
    GameWorld,
    audio::Sfx,
    deer::{Deer, DeerAnim, Mouth},
    player::LocalDeer,
};
use bevy::{light::NotShadowCaster, prelude::*};
use deer_shared::{
    protocol::Action,
    world::{WATER_LEVEL, wrap},
};
use std::collections::HashMap;

const LIFE: f32 = 1.6;
/// Between rings while wading at walking pace; faster movement makes them closer together.
const WADE_EVERY: f32 = 0.42;
const IDLE_EVERY: f32 = 2.2;

/// Start a ring on the water here (render space), `size` metres across when it fades.
#[derive(Event, Clone, Copy)]
pub struct RippleAt {
    pub at: Vec3,
    pub size: f32,
}

#[derive(Component)]
struct Ripple {
    age: f32,
    size: f32,
    material: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct RingMesh(Handle<Mesh>);

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, (wade, grow))
        .add_observer(spawn);
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    // A thin ring lying flat, one metre across; scaled as it spreads.
    let ring = Annulus::new(0.44, 0.5)
        .mesh()
        .resolution(40)
        .build()
        .rotated_by(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    commands.insert_resource(RingMesh(meshes.add(ring)));
}

fn spawn(
    ev: On<RippleAt>,
    mut commands: Commands,
    mesh: Res<RingMesh>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.92, 0.95, 1.0, 0.0),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.spawn((
        Ripple {
            age: 0.0,
            size: ev.size,
            material: material.clone(),
        },
        Mesh3d(mesh.0.clone()),
        MeshMaterial3d(material),
        Transform::from_translation(Vec3::new(ev.at.x, WATER_LEVEL + 0.015, ev.at.z))
            .with_scale(Vec3::splat(0.1)),
        NotShadowCaster,
    ));
}

fn grow(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Ripple, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (e, mut r, mut t) in &mut q {
        r.age += time.delta_secs();
        let k = r.age / LIFE;
        if k >= 1.0 {
            materials.remove(&r.material);
            commands.entity(e).despawn();
            continue;
        }
        // Spreads fast then slows; fades out as it goes.
        let spread = 1.0 - (1.0 - k).powi(3);
        t.scale = Vec3::splat(0.15 + r.size * spread);
        if let Some(mut m) = materials.get_mut(&r.material) {
            m.base_color.set_alpha(0.45 * (1.0 - k) * (1.0 - k));
        }
    }
}

type Wader<'a> = (
    Entity,
    &'a Transform,
    &'a DeerAnim,
    &'a Mouth,
    Has<LocalDeer>,
);

/// Every deer standing in water makes rings: as it walks (more often the faster it goes) and,
/// now and then, while it stands. Ghosts' wading steps also splash.
fn wade(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<GameWorld>,
    deer: Query<Wader, With<Deer>>,
    mut timers: Local<HashMap<Entity, f32>>,
) {
    let dt = time.delta_secs();
    timers.retain(|e, _| deer.contains(*e));
    for (e, t, anim, mouth, local) in &deer {
        let p = t.translation;
        let depth = WATER_LEVEL - world.0.height(wrap(p.x), wrap(p.z));
        if depth <= 0.01 {
            continue;
        }
        let timer = timers.entry(e).or_insert(0.0);
        *timer -= dt;
        if *timer > 0.0 {
            continue;
        }
        let moving = anim.speed > 0.3;
        if moving {
            *timer = WADE_EVERY * (1.9 / anim.speed.max(0.5)).clamp(0.35, 1.6);
            let jitter = Vec3::new(
                rand::random_range(-0.25..0.25),
                0.0,
                rand::random_range(-0.25..0.25),
            );
            commands.trigger(RippleAt {
                at: p + jitter,
                size: 1.1 + rand::random_range(0.0..0.5),
            });
            if !local {
                commands.trigger(Sfx::Splash(p));
            }
        } else if anim.action != Action::Drinking {
            *timer = IDLE_EVERY * rand::random_range(0.7..1.3);
            commands.trigger(RippleAt { at: p, size: 0.8 });
        } else {
            *timer = 1.1;
            // Someone else sipping (the player's own sips ring in `interact`).
            if !local && mouth.mouth.y < WATER_LEVEL + 0.15 {
                commands.trigger(RippleAt {
                    at: mouth.mouth,
                    size: 0.7,
                });
            }
        }
    }
}
