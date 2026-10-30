//! Development helpers, all off unless their environment variable is set:
//! - `DEER_CAM=x,y,z,tx,ty,tz` puts the camera at (x,y,z) looking at (tx,ty,tz).
//! - `DEER_SHOT=path.png` saves a screenshot after a few seconds and quits.

use crate::env::MainCamera;
use crate::wrap::Focus;
use bevy::{
    camera::RenderTarget,
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, save_to_disk},
    },
};

pub fn plugin(app: &mut App) {
    if std::env::var("DEER_COLLIDERS").is_ok() {
        app.add_systems(Update, draw_colliders);
    }
    if std::env::var("DEER_PROFILE").is_ok() {
        app.add_systems(Update, profile);
    }
    if std::env::var("DEER_FPS").is_ok() {
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ));
    }
    if std::env::var("DEER_AUTOPLAY").is_ok() {
        app.add_systems(Update, autoplay.run_if(in_state(crate::AppState::Title)));
    }
    if std::env::var("DEER_CLICKS").is_ok() {
        app.add_systems(PreUpdate, scripted_clicks.after(bevy::input::InputSystems));
    }
    if std::env::var("DEER_SOLO").is_ok() {
        app.add_systems(Startup, solo);
    }
    if std::env::var("DEER_CAM").is_ok() {
        app.add_systems(Update, (fixed_camera, hide_title));
    }
    if let Ok(path) = std::env::var("DEER_SHOT") {
        app.insert_resource(ShotPath(path))
            .add_systems(
                PostStartup,
                offscreen_target.after(crate::render_scale::ScaleSetup),
            )
            .add_systems(Update, take_shot);
    }
}

#[derive(Resource)]
struct ShotPath(String);

/// Renders into an image instead of the window: a window in the background presents nothing,
/// so window screenshots come out black.
#[derive(Resource)]
struct ShotImage(Handle<Image>);

fn offscreen_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    // The camera that produces the final picture: the UI camera when the 3D view is scaled.
    cam: Single<Entity, With<IsDefaultUiCamera>>,
) {
    let (w, h) = shot_size();
    let mut image = Image::new_target_texture(w, h, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.size = Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    image.texture_descriptor.dimension = TextureDimension::D2;
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = images.add(image);
    commands
        .entity(*cam)
        .insert(RenderTarget::Image(handle.clone().into()));
    commands.insert_resource(ShotImage(handle));
}

fn parse_cam() -> Option<(Vec3, Vec3)> {
    let v: Vec<f32> = std::env::var("DEER_CAM")
        .ok()?
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    (v.len() == 6).then(|| (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5])))
}

/// With a fixed camera the title screen is only in the way.
fn hide_title(mut q: Query<&mut Visibility, With<crate::ui::TitleRoot>>) {
    for mut v in &mut q {
        *v = Visibility::Hidden;
    }
}

fn fixed_camera(mut focus: ResMut<Focus>, mut cam: Single<&mut Transform, With<MainCamera>>) {
    if let Some((eye, target)) = parse_cam() {
        **cam = Transform::from_translation(eye).looking_at(target, Vec3::Y);
        focus.0 = eye;
    }
}

fn take_shot(
    mut commands: Commands,
    time: Res<Time>,
    path: Res<ShotPath>,
    image: Res<ShotImage>,
    mut taken: Local<usize>,
    mut exit: MessageWriter<AppExit>,
) {
    // `DEER_SHOT_SECS=8` or a list `8,8.1,8.2` (then files get `_0`, `_1`, ... suffixes).
    let t = time.elapsed_secs();
    let times: Vec<f32> = std::env::var("DEER_SHOT_SECS")
        .unwrap_or_else(|_| "6".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if let Some(&at) = times.get(*taken) {
        if t > at {
            let file = if times.len() > 1 {
                path.0.replace(".png", &format!("_{}.png", *taken))
            } else {
                path.0.clone()
            };
            commands
                .spawn(Screenshot::image(image.0.clone()))
                .observe(save_to_disk(file));
            *taken += 1;
        }
    } else if t > times.last().copied().unwrap_or(6.0) + 4.0 {
        exit.write(AppExit::Success);
    }
}

/// `DEER_SOLO=1`: straight into the world without the title screen or a server.
fn solo(
    mut commands: Commands,
    assets: Res<crate::deer::DeerAssets>,
    world: Res<crate::GameWorld>,
    mut look: ResMut<crate::player::Look>,
    mut next: ResMut<NextState<crate::AppState>>,
) {
    let stag = std::env::var("DEER_STAG").is_ok();
    let at = spawn_override().unwrap_or_else(|| world.0.spawn_point(0));
    crate::player::spawn_local(&mut commands, &assets, &world.0, stag, at, &mut look);
    next.set(crate::AppState::Playing);
}

/// `DEER_AUTOPLAY=1`: presses play on the title screen after a moment.
fn autoplay(mut commands: Commands, time: Res<Time>, mut done: Local<bool>) {
    if !*done && time.elapsed_secs() > 1.5 {
        *done = true;
        crate::ui::press_play(&mut commands);
    }
}

/// `DEER_AT=x,z` puts the player's deer there instead of where the server says.
pub fn spawn_override() -> Option<[f32; 2]> {
    let v: Vec<f32> = std::env::var("DEER_AT")
        .ok()?
        .split(',')
        .filter_map(|x| x.parse().ok())
        .collect();
    (v.len() == 2).then(|| [v[0], v[1]])
}

/// `DEER_NAME=...` names the deer instead of the title screen's box.
pub fn name_override() -> Option<String> {
    std::env::var("DEER_NAME").ok()
}

/// `DEER_YAW=radians` turns the player's deer (and camera) on spawn.
pub fn yaw_override() -> Option<f32> {
    std::env::var("DEER_YAW").ok()?.parse().ok()
}

/// Whether scripted clicks are on: the game then behaves as if the mouse were captured,
/// without capturing the real one.
pub fn scripted_clicks_on() -> bool {
    std::env::var("DEER_CLICKS").is_ok()
}

/// `DEER_CLICKS=t1,t2,...` presses the left mouse button at those times (seconds), as if the
/// cursor were captured, and `DEER_PITCH` tilts the view: for testing eating and drinking.
fn scripted_clicks(
    time: Res<Time>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut look: ResMut<crate::player::Look>,
    mut next: Local<usize>,
) {
    let times: Vec<f32> = std::env::var("DEER_CLICKS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    let t = time.elapsed_secs();
    // `DEER_HOLD=w` walks forward from 3 s on, `DEER_HOLD=run` gallops.
    if t > 3.0 {
        match std::env::var("DEER_HOLD").as_deref() {
            Ok("w") => keys.press(KeyCode::KeyW),
            Ok("run") => {
                keys.press(KeyCode::KeyW);
                keys.press(KeyCode::ShiftLeft);
            }
            _ => {}
        }
        if let Some(y) = knob("DEER_LOOK_YAW") {
            look.yaw = y;
        }
        // `DEER_LOOK_AT=t,yaw,pitch`: swing the view there from time t on.
        if let Ok(v) = std::env::var("DEER_LOOK_AT") {
            let v: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 && t > v[0] {
                look.yaw = v[1];
                look.pitch = v[2];
                return;
            }
        }
        if let Some(p) = std::env::var("DEER_PITCH")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            look.pitch = p;
        }
    }
    if let Some(&at) = times.get(*next)
        && t > at
    {
        *next += 1;
        mouse.press(MouseButton::Left);
    } else {
        mouse.release(MouseButton::Left);
    }
}

/// `DEER_RES=WxH`: size of the off-screen render (default 1600x900).
pub fn shot_size() -> (u32, u32) {
    std::env::var("DEER_RES")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1600, 900))
}

/// `DEER_OFF=a,b,...` switches features off for profiling: shadows, ssr, taa, bloom, atmo,
/// trees, detail, wind, beams, ambient, fog.
pub fn off(feature: &str) -> bool {
    std::env::var("DEER_OFF")
        .map(|v| v.split(',').any(|f| f.trim() == feature))
        .unwrap_or(false)
}

/// `DEER_PROFILE=1`: averages the frame time from 10 s to 16 s after start, prints it, quits.
fn profile(
    time: Res<Time<Real>>,
    entities: Query<Entity>,
    meshes: Query<(), With<Mesh3d>>,
    mut samples: Local<Vec<f32>>,
    mut exit: MessageWriter<AppExit>,
) {
    let t = time.elapsed_secs();
    if t > 10.0 {
        samples.push(time.delta_secs() * 1000.0);
    }
    if t > 16.0 {
        let mut s = samples.clone();
        s.sort_by(|a, b| a.total_cmp(b));
        let avg = s.iter().sum::<f32>() / s.len().max(1) as f32;
        let p50 = s.get(s.len() / 2).copied().unwrap_or(0.0);
        let p95 = s.get(s.len() * 95 / 100).copied().unwrap_or(0.0);
        println!(
            "PROFILE off={} avg {avg:.2} ms ({:.0} fps) p50 {p50:.2} ms p95 {p95:.2} ms, {} entities, {} meshes",
            std::env::var("DEER_OFF").unwrap_or_default(),
            1000.0 / avg,
            entities.iter().len(),
            meshes.iter().len()
        );
        exit.write(AppExit::Success);
    }
}

/// `DEER_CASCADES=n,first,max`: shadow cascade layout, for profiling.
pub fn cascades() -> Option<(usize, f32, f32)> {
    let v: Vec<f32> = std::env::var("DEER_CASCADES")
        .ok()?
        .split(',')
        .filter_map(|x| x.parse().ok())
        .collect();
    (v.len() == 3).then(|| (v[0] as usize, v[1], v[2]))
}

/// `DEER_SHADOW_SIZE=px`: shadow map size per cascade, for profiling.
pub fn shadow_size() -> Option<usize> {
    std::env::var("DEER_SHADOW_SIZE").ok()?.parse().ok()
}

/// A numeric tuning knob from the environment, for profiling.
pub fn knob(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.parse().ok()
}

/// `DEER_COLLIDERS=1`: draws the trunk collision circles near the player.
fn draw_colliders(
    mut gizmos: Gizmos,
    focus: Res<Focus>,
    world: Res<crate::GameWorld>,
    shapes: Option<Res<crate::vegetation::TreeShapes>>,
    mouths: Query<&crate::deer::Mouth>,
) {
    for m in &mouths {
        gizmos.sphere(
            Isometry3d::from_translation(m.mouth),
            0.03,
            Color::srgb(0.2, 1.0, 0.3),
        );
    }
    let Some(shapes) = shapes else { return };
    let f = focus.0;
    world.0.trees_near(
        deer_shared::world::wrap(f.x),
        deer_shared::world::wrap(f.z),
        40.0,
        |t| {
            let Some(s) = shapes.0.get(t.id as usize).copied().flatten() else {
                return;
            };
            for ring in s.rings() {
                let p = crate::wrap::nearest(Vec3::new(ring.pos[0], t.y + ring.y, ring.pos[1]), f);
                gizmos.circle(
                    Isometry3d::new(p, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                    ring.r,
                    Color::srgb(1.0, 0.1, 0.6),
                );
            }
        },
    );
}
