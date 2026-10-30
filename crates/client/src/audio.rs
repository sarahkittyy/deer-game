//! Sound: a morning ambience bed (birdsong, wind in the leaves, faint crickets), water lapping at
//! the nearest shore, and short spatial sounds for everything that happens in the world.

use crate::{GameWorld, env::MainCamera, wrap::Focus};
use bevy::{
    audio::{AudioPlugin, PlaybackMode, SpatialScale, Volume},
    prelude::*,
};
use deer_shared::world::{WATER_LEVEL, wrap_delta};

/// A one-shot sound. Positions are where it is drawn (near the camera), not wrapped.
#[derive(Event, Clone, Copy, Debug)]
pub enum Sfx {
    Click,
    Eat(Vec3),
    Drink(Vec3),
    Step(Vec3),
    /// A step in shallow water.
    Splash(Vec3),
    /// The deer sniffing the air: barely there.
    Sniff(Vec3),
    Land(Vec3),
    Bird(Vec3),
    Frog(Vec3),
    Rustle(Vec3),
    /// A wolf's paw; `true` at a gallop (louder).
    WolfStep(Vec3, bool),
    WolfBark(Vec3),
    WolfGrowl(Vec3),
    /// The pounce: a loud bark and a snarl.
    WolfAttack(Vec3),
    /// A body hitting something hard: the wolf's hit, a trunk, the ground after a long fall.
    Thud(Vec3, f32),
}

#[derive(Resource)]
struct Sounds {
    click: Handle<AudioSource>,
    eat: Vec<Handle<AudioSource>>,
    drink: Vec<Handle<AudioSource>>,
    step: Vec<Handle<AudioSource>>,
    splash: Vec<Handle<AudioSource>>,
    sniff: Vec<Handle<AudioSource>>,
    land: Handle<AudioSource>,
    bird: Vec<Handle<AudioSource>>,
    frog: Vec<Handle<AudioSource>>,
    rustle: Vec<Handle<AudioSource>>,
    wolf_paw: Vec<Handle<AudioSource>>,
    wolf_bark: Vec<Handle<AudioSource>>,
    wolf_growl: Vec<Handle<AudioSource>>,
    wolf_attack: Vec<Handle<AudioSource>>,
    wolf_snarl: Vec<Handle<AudioSource>>,
}

/// The looping emitter that sits on the nearest lake shore.
#[derive(Component)]
struct Lapping;

pub fn audio_plugin() -> AudioPlugin {
    AudioPlugin {
        // Rodio's spatial falloff is 1/d²; at this scale sounds are full volume out to ~10 m.
        default_spatial_scale: SpatialScale::new(0.1),
        ..default()
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(PostStartup, add_listener)
        .add_systems(Update, follow_shore)
        .add_observer(play);
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    let many = |prefix: &str, n: usize| -> Vec<Handle<AudioSource>> {
        (1..=n)
            .map(|i| assets.load(format!("audio/{prefix}_{i}.ogg")))
            .collect()
    };
    commands.insert_resource(Sounds {
        click: assets.load("audio/ui_click_2.ogg"),
        eat: many("eat", 3),
        drink: many("drink", 2),
        step: many("step", 6),
        splash: many("splash", 6),
        sniff: many("sniff", 1),
        land: assets.load("audio/land.ogg"),
        bird: many("bird_chirp", 7),
        frog: many("frog", 5),
        rustle: many("rustle", 4),
        wolf_paw: many("wolf_paw", 6),
        wolf_bark: many("wolf_bark", 6),
        wolf_growl: many("wolf_growl", 4),
        wolf_attack: many("wolf_attack", 4),
        wolf_snarl: many("wolf_snarl", 2),
    });

    let bed = |path: &str, vol: f32| {
        (
            AudioPlayer::new(assets.load(path.to_string())),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(vol)),
        )
    };
    commands.spawn(bed("audio/ambience_morning_1.ogg", 0.55));
    commands.spawn(bed("audio/wind_leaves_loop.ogg", 0.35));
    commands.spawn(bed("audio/crickets_loop.ogg", 0.12));
    commands.spawn((
        Lapping,
        AudioPlayer::new(assets.load("audio/water_lapping_loop.ogg")),
        PlaybackSettings::LOOP
            .with_volume(Volume::Linear(0.5))
            .with_spatial(true),
        Transform::from_xyz(0.0, -1000.0, 0.0),
    ));
}

fn add_listener(mut commands: Commands, cam: Single<Entity, With<MainCamera>>) {
    commands.entity(*cam).insert(SpatialListener::new(0.3));
}

/// Moves the lapping sound to the closest point of the closest lake's shore.
fn follow_shore(
    world: Res<GameWorld>,
    focus: Res<Focus>,
    mut lap: Single<&mut Transform, With<Lapping>>,
) {
    let f = focus.0;
    let mut best: Option<(f32, Vec3)> = None;
    for lake in &world.0.lakes {
        let d = Vec2::new(
            wrap_delta(lake.center[0] - f.x),
            wrap_delta(lake.center[1] - f.z),
        );
        let len = d.length();
        let shore = if len > lake.radius {
            Vec2::new(f.x, f.z) + d * (1.0 - lake.radius / len)
        } else {
            Vec2::new(f.x, f.z)
        };
        let dist = (len - lake.radius).max(0.0);
        if best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, Vec3::new(shore.x, WATER_LEVEL, shore.y)));
        }
    }
    if let Some((_, p)) = best {
        lap.translation = p;
    }
}

fn pick(v: &[Handle<AudioSource>]) -> &Handle<AudioSource> {
    &v[rand::random_range(0..v.len())]
}

fn play(ev: On<Sfx>, mut commands: Commands, sounds: Res<Sounds>) {
    let mut spawn = |handle: &Handle<AudioSource>, vol: f32, at: Option<Vec3>, speed: f32| {
        let settings = PlaybackSettings {
            mode: PlaybackMode::Despawn,
            volume: Volume::Linear(vol),
            speed,
            spatial: at.is_some(),
            ..default()
        };
        let mut e = commands.spawn((AudioPlayer::new(handle.clone()), settings));
        if let Some(p) = at {
            e.insert(Transform::from_translation(p));
        }
    };
    let (handle, vol, at, speed) = match *ev {
        Sfx::Click => (&sounds.click, 0.5, None, 1.0),
        Sfx::Eat(p) => (
            pick(&sounds.eat),
            0.8,
            Some(p),
            rand::random_range(0.95..1.1),
        ),
        Sfx::Drink(p) => (pick(&sounds.drink), 0.3, Some(p), 1.0),
        Sfx::Step(p) => (
            pick(&sounds.step),
            0.16,
            Some(p),
            rand::random_range(0.9..1.15),
        ),
        Sfx::Sniff(p) => (
            pick(&sounds.sniff),
            0.25,
            Some(p),
            rand::random_range(0.95..1.05),
        ),
        Sfx::Splash(p) => (
            pick(&sounds.splash),
            0.3,
            Some(p),
            rand::random_range(0.9..1.12),
        ),
        Sfx::Land(p) => (&sounds.land, 0.35, Some(p), 1.0),
        Sfx::Bird(p) => (
            pick(&sounds.bird),
            0.5,
            Some(p),
            rand::random_range(0.95..1.08),
        ),
        Sfx::Frog(p) => (
            pick(&sounds.frog),
            0.55,
            Some(p),
            rand::random_range(0.9..1.1),
        ),
        Sfx::Rustle(p) => (pick(&sounds.rustle), 0.3, Some(p), 1.0),
        Sfx::WolfStep(p, gallop) => (
            pick(&sounds.wolf_paw),
            if gallop { 0.11 } else { 0.06 },
            Some(p),
            rand::random_range(0.85..1.1),
        ),
        // Wolves are kept under 1: louder, a close one clips and drowns the panning that says
        // where it is. Barks still carry (falloff starts at 10 m).
        Sfx::WolfBark(p) => (
            pick(&sounds.wolf_bark),
            0.9,
            Some(p),
            rand::random_range(0.88..1.0),
        ),
        Sfx::WolfGrowl(p) => (
            pick(&sounds.wolf_growl),
            0.6,
            Some(p),
            rand::random_range(0.9..1.05),
        ),
        Sfx::WolfAttack(p) => {
            spawn(pick(&sounds.wolf_snarl), 0.6, Some(p), 1.0);
            (
                pick(&sounds.wolf_attack),
                1.0,
                Some(p),
                rand::random_range(0.9..1.0),
            )
        }
        Sfx::Thud(p, strength) => (
            &sounds.land,
            (0.3 + strength * 0.7).min(1.4),
            Some(p),
            rand::random_range(0.7..0.85),
        ),
    };
    spawn(handle, vol, at, speed);
}
