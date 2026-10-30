//! Dynamic resolution. The 3D view renders into an off-screen image with fewer pixels than the
//! window (about a 1440p screen's worth at most), is sharpened, and is stretched over the
//! window; the UI draws on top at full resolution so text stays crisp. If frames still run
//! long, the scale steps down; when there is room again, it steps back up.

use crate::env::MainCamera;
use bevy::{
    anti_alias::contrast_adaptive_sharpening::ContrastAdaptiveSharpening,
    camera::RenderTarget,
    prelude::*,
    render::render_resource::{Extent3d, TextureFormat, TextureUsages},
    window::{PrimaryWindow, WindowResized},
};

/// The most pixels the 3D view renders at: a 2560 x 1440 screen.
const MAX_PIXELS: f32 = 2560.0 * 1440.0;
const MIN_SCALE: f32 = 0.5;
/// Frame time (ms) above which the scale steps down, and below which it may step back up.
const SLOW_MS: f32 = 19.0;
const FAST_MS: f32 = 12.0;
/// How long frames must stay slow or fast before the scale changes.
const SETTLE_SECS: f32 = 2.5;
const STEP: f32 = 0.1;

#[derive(Resource)]
pub struct RenderScale {
    /// Fraction of the window's pixels (per axis) the 3D view renders at.
    pub scale: f32,
    /// The scale chosen for this window size, which dynamic changes never exceed.
    ceiling: f32,
    image: Handle<Image>,
    slow_for: f32,
    fast_for: f32,
    avg_ms: f32,
}

#[derive(Component)]
struct Upscaled;

#[derive(Component)]
struct UpscaleCamera;

pub fn plugin(app: &mut App) {
    if std::env::var("DEER_NO_SCALE").is_ok() {
        return;
    }
    app.add_systems(PostStartup, setup.in_set(ScaleSetup))
        .add_systems(Update, (adapt, resize).chain())
        .add_systems(
            PostUpdate,
            follow_main_camera.before(TransformSystems::Propagate),
        );
}

/// Bevy's distance culling (`VisibilityRange`) also measures from this camera, so it has to
/// stand where the 3D camera stands; otherwise far-off trees vanish from the shadow maps.
fn follow_main_camera(
    main: Single<&Transform, (With<MainCamera>, Without<UpscaleCamera>)>,
    mut ui: Single<&mut Transform, With<UpscaleCamera>>,
) {
    ui.translation = main.translation;
}

/// Ordering label for the debug screenshot path, which redirects the final image.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScaleSetup;

/// The size of what is finally shown: the window, or the debug screenshot.
fn output_size(window: &Window) -> UVec2 {
    if std::env::var("DEER_SHOT").is_ok() {
        let (w, h) = crate::debug::shot_size();
        return UVec2::new(w, h);
    }
    UVec2::new(
        window.physical_width().max(1),
        window.physical_height().max(1),
    )
}

fn ceiling_for(window: &Window) -> f32 {
    if let Some(s) = crate::debug::knob("DEER_SCALE") {
        return s.clamp(0.25, 1.0);
    }
    let o = output_size(window).as_vec2();
    (MAX_PIXELS / (o.x * o.y)).sqrt().clamp(MIN_SCALE, 1.0)
}

fn target_size(window: &Window, scale: f32) -> UVec2 {
    let o = output_size(window).as_vec2();
    UVec2::new(
        ((o.x * scale).round() as u32).max(64),
        ((o.y * scale).round() as u32).max(64),
    )
}

fn setup(
    mut commands: Commands,
    window: Single<&Window, With<PrimaryWindow>>,
    cam: Single<Entity, With<MainCamera>>,
    mut images: ResMut<Assets<Image>>,
) {
    let scale = ceiling_for(&window);
    let size = target_size(&window, scale);
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = images.add(image);

    let mut cam = commands.entity(*cam);
    cam.insert(RenderTarget::Image(handle.clone().into()))
        .remove::<IsDefaultUiCamera>();
    // Sharpen before stretching: most of the crispness lost to the lower resolution comes back
    // for little cost. (Not on WebGL2, where Bevy's sharpening pipeline fails to validate.)
    if !crate::env::lite() {
        cam.insert(ContrastAdaptiveSharpening {
            enabled: true,
            sharpening_strength: 0.45,
            denoise: false,
        });
    }
    // Draws the stretched 3D view, then the UI over it, at the window's full resolution.
    commands.spawn((
        UpscaleCamera,
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        Msaa::Off,
        IsDefaultUiCamera,
    ));
    commands.spawn((
        Upscaled,
        ImageNode::new(handle.clone()),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        GlobalZIndex(i32::MIN),
        Pickable::IGNORE,
    ));
    commands.insert_resource(RenderScale {
        scale,
        ceiling: scale,
        image: handle,
        slow_for: 0.0,
        fast_for: 0.0,
        avg_ms: 0.0,
    });
    info!(
        "rendering the 3D view at {}x{} ({:.0}%)",
        size.x,
        size.y,
        scale * 100.0
    );
}

/// Steps the scale down while frames are slow, and back up (never past the ceiling) when
/// they are fast again.
fn adapt(time: Res<Time<Real>>, mut rs: ResMut<RenderScale>) {
    let ms = time.delta_secs() * 1000.0;
    let dt = time.delta_secs();
    // Ignore hitches (loading, alt-tab): they say nothing about steady-state cost.
    if ms > 100.0 {
        return;
    }
    rs.avg_ms += (ms - rs.avg_ms) * (1.0 - (-dt * 2.0).exp());
    if rs.avg_ms > SLOW_MS {
        rs.slow_for += dt;
        rs.fast_for = 0.0;
    } else if rs.avg_ms < FAST_MS {
        rs.fast_for += dt;
        rs.slow_for = 0.0;
    } else {
        rs.slow_for = 0.0;
        rs.fast_for = 0.0;
    }
    if rs.slow_for > SETTLE_SECS && rs.scale > MIN_SCALE {
        rs.scale = (rs.scale - STEP).max(MIN_SCALE);
        rs.slow_for = 0.0;
    } else if rs.fast_for > SETTLE_SECS * 2.0 && rs.scale < rs.ceiling {
        rs.scale = (rs.scale + STEP).min(rs.ceiling);
        rs.fast_for = 0.0;
    }
}

/// Keeps the off-screen image matching the window size and the current scale.
fn resize(
    mut resized: MessageReader<WindowResized>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut rs: ResMut<RenderScale>,
    mut images: ResMut<Assets<Image>>,
) {
    if resized.read().count() > 0 {
        let ceiling = ceiling_for(&window);
        rs.ceiling = ceiling;
        rs.scale = rs.scale.min(ceiling).max(MIN_SCALE.min(ceiling));
    }
    let want = target_size(&window, rs.scale);
    let Some(image) = images.get(&rs.image) else {
        return;
    };
    let have = UVec2::new(image.width(), image.height());
    if have != want {
        if let Some(mut image) = images.get_mut(&rs.image) {
            image.resize(Extent3d {
                width: want.x,
                height: want.y,
                depth_or_array_layers: 1,
            });
        }
        debug!(
            "render scale {:.0}%: {}x{}",
            rs.scale * 100.0,
            want.x,
            want.y
        );
    }
}
